# Review: python-security pack

Reviewer's audit of the seven rules in `packs/python-security/` (rules, cases, README "Rule details",
`NOTES-python-security.md`). Nothing in the rules, cases or `pack.json` was changed; the verdicts are the output.
Every count here was measured for this review; the README's corpus figures are the writers' and were not copied.

## 1. Summary

| Rule | Verdict | Proposed mode / on | Corpus matches (repos with matches) | Read precision | Reason |
|---|---|---|---|---|---|
| `python-security-sql-injection` | rework | steer / on | 0 (0) of 23 run | n/a (nothing to read) | Silent in mature code; 4 false-positive shapes on validated ints/choices/patterns; misses asyncpg `fetch*` and a whole client fragment in `order_by(text(sort))` |
| `python-security-command-injection` | rework | steer / on | 0 (0) of 23 run | n/a | A helper named `run_healthcheck` counts as the validator of the host it pings; a `ChoiceField` program is reported |
| `python-security-code-injection` | rework | steer / on | 0 (0) of 23 run | n/a | `pickle.loads(zlib.decompress(body))`-style decoders end the taint; Django `Template` message overstates the risk |
| `python-security-jwt-unverified` | rework | steer / on once it finishes on mid-size repos | 4 (2) of 15 run; engine failed on 5 | 4/4 (100%) | Finds Lemur's published advisory; exhausts the engine's content store on open-webui, freqtrade, pretix, netbox, addons-server; 2 false-positive shapes in the cx |
| `python-security-path-traversal` | rework | log / on | 12 (1) of 28 run | 7/12 (58%) | Path parameters with a fixed suffix and extension-only values are reported; PIL/pandas/archive reads missed |
| `python-security-ssrf` | rework | log / on | 10 (5) of 29 run | 2/10 (20%) | Confirmed f-string bug (host fixed by settings still "client-controlled"); guarded transports and admin-only routes reported |
| `python-security-orm-lookup-injection` | rework | log / on; steer after the staff exemption | 3 (2) of 15 run | 2/3 (67%) | Real leaks in lookit; misses helper receivers and free-text `validated_data`; staff-only admin view reported |

Verdict counts: keep 0, rework 7, cut 0. No rule is worthless: each targets a real, sourced vulnerability class and
none duplicates a linter in this lineup. None can ship as is either: each has at least one confirmed false-positive
shape or miss on a common idiom, and two (ssrf, jwt-unverified) have defects in the rule itself (a logic bug, a
resource blow-up).

Reading order: `pack.json` lists the rules alphabetically. Read them by worst outcome: `sql-injection`,
`command-injection`, `code-injection` (the injection trio), then `jwt-unverified`, `path-traversal`, `ssrf`, and last
`orm-lookup-injection`. Sections 2 to 8 follow that order.

Two defects of the shared taint library, `rules/lib/security.rerail`, cause most of the false positives and misses
below (section 9):

- A value a DRF serializer, a Django form, a FastAPI `Query(pattern=…)` or a pydantic `Literal` field has validated
  stays tainted, so an `IntegerField` or `ChoiceField` value spliced into SQL, or used as a program, is reported.
- Any call whose name *contains* `check`, `valid`, `safe`, `allow`, `verif`, … counts as a check of every name passed
  to it, so `host = request.GET["host"]; run_healthcheck(host)` clears the host the helper pings through a shell, and
  Django's `URLValidator` clears an SSRF.

## 2. `python-security-sql-injection`

**Value.** Request text spliced into SQL is the textbook critical bug (data theft, data loss). The sources hold up: I
opened GHSA-pwh4-6r3m-j2rf (pyload-ng, "SQL Injection via API /json/add_package"), GHSA-r75f-5x8p-qvmc (LiteLLM, "SQL
Injection in Proxy API key verification"); the Django raw-SQL and SQLAlchemy pages say what the README quotes. How
often a team hits it: rarely in mature Django/FastAPI code (the ORM parameterises), which the corpus confirms
(below); when it does happen it is critical.

**Correctness.** The logic follows `sec_built` (a request value spliced by f-string, `%`, `+`, `format`, `join`)
into the SQL-text argument only, with a check-aware piece filter. Sound design; the params position is never a sink.
Counterexamples (`/tmp/review-python-security/sqli/`, run `review-python-security-sql-injection-cx`):

| Shape | Expected | Result |
|---|---|---|
| DRF serializer `IntegerField` and `ChoiceField(["ASC","DESC"])` values in `f"… ORDER BY id {d} LIMIT {n}"` (`shop/views.py:21`) | silent: both validated | **reported** (false positive) |
| Django form `IntegerField` from `cleaned_data` in `"… LIMIT %d" % n` (`shop/views.py:29`) | silent | **reported** (false positive) |
| FastAPI `order: str = Query("asc", pattern="^(asc|desc)$")` in `text(f"… ORDER BY id {order}")` (`api/routes.py:18`) | silent | **reported** (false positive) |
| Pydantic body field `sort_dir: Literal["asc","desc"]` in `text(f"…{body.sort_dir}")` (`api/routes.py:23`) | silent | **reported** (false positive) |
| `", ".join("%s" for _ in ids)` placeholder list with `ids` as params | silent | silent (correct) |
| asyncpg `await conn.fetch(f"… '{name}'")` in a FastAPI route (`api/routes.py:28`) | reported | **missed**: `fetch`, `fetchrow`, `fetchval` (asyncpg, `databases`) are not sinks |
| `select(...).order_by(text(sort))` with `sort` a route parameter (`api/routes.py:33`) | reported | **missed**: a client string used whole as a SQL fragment is treated like an "SQL lab" query |
| `parts.append(f"AND name = '{q}'")` then `" ".join(parts)` | reported | missed (documented gap) |
| Django URL kwarg `def view(request, slug)` and `self.kwargs["name"]` in an f-string query | reported | missed (documented gap) |
| `pd.read_sql(f"…", engine)` | reported | missed (documented gap) |
| route parameter `name: str` in `text(f"… '{name}'")` (`api/routes.py:43`) | reported | reported |

The "client query run as written" exemption is right for a SQL-lab feature (`cursor.execute(request.POST["sql"])`)
but wrong for a fragment: `order_by(text(sort))`, `where(text(clause))`, `literal_column(col)` take a whole client
string into the middle of a server-built query. Those are classic sort-column injections.

**Corpus.** 0 matches. On every web repository it ran on (section 11 lists them, FastAPI, Django and Flask). There is
nothing to read, so precision has no denominator; what the corpus shows is that the rule is silent in mature code,
which is what a team wants from an insurance rule.

**Overlap.** Ruff S608 / Bandit B608 flag every SQL-looking formatted string; neither is ported in the `python` pack
(it ports B610 `django-extra-used`, not B608/B611), so in this lineup nothing else covers raw-SQL injection for
Django/DB-API. Inside the lineup, `sqlalchemy-practice-text-interpolation` (sqlalchemy pack, log) reports every
`text()`/`exec_driver_sql()` built from any non-constant value, which is a superset of this rule's SQLAlchemy sinks:
both fire on the same `text(f"…{request_value}…")` line. This rule adds the request source (a `steer`-worthy signal
where the other rule is a style-and-safety `log` rule), the Django and DB-API sinks, and the cross-file helper
walk. Keep both; say in each README that the other exists. Semgrep has request-source rules for Django/Flask in one
function only.

**Text.** Description (121 characters) lists implementation tokens ("an f-string, %, +, format() or join()"),
against criterion 3; suggested: "Pass request data to SQL as query parameters, never spliced into the SQL text." The
message (182 characters) names the change. Its exemption "Not needed for a value cast to int." is dead text: an
`int()` cast already stops the taint, so no reported line is int-cast. Replace with the case that is reported and
fine: "Leave it if the value is already limited to fixed choices (an int field, a choice field, an allow-list)."

**Verdict: rework**, `steer`, on. Changes: (1) stop the taint at validated typed values: a DRF serializer's or Django
form's field whose declared type is `IntegerField`, `FloatField`, `DecimalField`, `BooleanField`, `UUIDField`,
`DateField`/`DateTimeField` or `ChoiceField`/`TypedChoiceField`, FastAPI parameters with `pattern=`/`regex=` or an
`Enum`/`Literal`/`int` annotation, and pydantic model fields typed `Literal`, `Enum`, `int`, `UUID`; (2) add
asyncpg/`databases` sinks (`fetch`, `fetchrow`, `fetchval`, `fetch_all`, `fetch_one`, `execute` on an asyncpg
connection or pool); (3) report a client string used whole inside `text()`, `literal_column()`, `extra(order_by=…)`
when the call is an argument of `order_by`/`where`/`filter`/`having` (the SQL-lab exemption stays for a whole
statement passed to `execute`); (4) the description and message edits above. On: zero matches on the corpus means
zero noise for a team, and every read case is a real injection.

## 3. `python-security-command-injection`

**Value.** Remote command execution. Sources hold up: GHSA-g5mq-prx7-c588 (motionEye "RCE in add_camera … unsafe
command execution"), GHSA-wwv8-cqpr-vx3m (Modoboa "OS Command Injection"), GHSA-79h8-gxhq-q3jg (lollms
`create_conda_env`), GHSA-w48f-wwwf-f5fr (pyLoad client-set executable): all opened. Frequency in web apps is low;
the corpus has no request-fed shell call.

**Correctness.** The shell-flag reading (`shell=` through names, parameters, `**kw` dicts) and the program-position
reading are careful. Counterexamples (`/tmp/review-python-security/cmdi/`, run
`review-python-security-command-injection-cx`):

| Shape | Expected | Result |
|---|---|---|
| `host = request.GET["host"]; git.run_healthcheck(host)`, helper runs `check_output(f"ping -c1 {host}", shell=True)` (`ops/git.py:9`) | reported | **missed**: `run_healthcheck` contains `check`, so `lib/security` counts `host` as validated in the view and the helper's parameter is clean |
| `git.checkout_branch(request.POST["branch"])`, helper runs `f"git checkout {name}"` with a shell (`ops/git.py:5`) | reported | reported: an argument that reads only `request` is never "checked"; the same call with a bound name would be missed like the line above |
| `if host in BLOCKED: raise` then `os.system(f"ping -c1 {host}")` (`ops/views.py:31`) | reported | missed (documented: any `in` test counts) |
| `subprocess.run([s.validated_data["tool"], "-h"])` with `tool = ChoiceField(["ls","df"])` (`ops/views.py:38`) | silent: the choice is a fixed allow-list | **reported** (false positive) |
| `["sh", "-c", 'echo "$1"', "_", request.GET["msg"]]`: the safe positional-argument idiom | silent | silent (correct) |
| `" ".join(["tar", "czf", out, request.FILES["f"].name])` under `shell=True` (`ops/views.py:48`) | reported | reported |
| Django URL kwarg in `os.system(f"convert {name} …")`; a value stored in `request.session` and run later | reported | missed (documented gaps) |

The validator-name heuristic is the real defect: `check` matches `run_healthcheck`, `checkout`, `spellcheck`,
`check_disk`; `safe`, `valid`, `allow` match just as loosely. For a rule about running commands, a function that
itself runs the command must never count as the check. Section 9 has the library-level fix.

**Corpus.** 0 matches on every repository it ran on (section 11). No precision figure.

**Overlap.** Ruff S602/S604/S605/S607 and Bandit B602/B605 flag every non-literal shell command; the `python` pack
ports neither (it ports B606 `start-process-with-no-shell` and B609 wildcard injection). Pylint has nothing. This rule
adds the request source, follows wrappers across files, trusts `shlex.quote`, and reads `shell` through `**kwargs`
(the Modoboa shape both linters miss).

**Text.** Description is good (113). Messages (195 and 184) name the change; "unless a check elsewhere fixes it" is
vague: say "unless the value was checked against a fixed allow-list before this call". `ShellCommand`'s "or
shlex.quote() the value" is correct only for POSIX shells; the README notes it, the message does not; acceptable.

**Verdict: rework**, `steer`, on. Changes: (1) a function that reaches a command sink (directly or through the run's
calls) is never a validator, and a name is checked only by a call whose name *starts* with a validator verb
(`is_`, `validate`, `check_`, `ensure_`, `sanitize`, `clean`), not by any call containing the substring; (2) treat a
`ChoiceField`/`TypedChoiceField`/`Literal`/`Enum` value as clean (the same change as sql-injection); (3) the message
edit. Zero corpus noise supports on.

## 4. `python-security-code-injection`

**Value.** Remote code execution through `eval`, pickle, unsafe YAML and template source. Sources hold up:
GHSA-xv5p-fjw5-vrj6 (fugue "RCE by Pickle Deserialization via FlaskRPCServer"), GHSA-6wvf-77m9-58rm (LiteLLM "server-side
template injection in the /prompts/test endpoint"), GHSA-4r7v-whpg-8rx3 (changedetection.io "SSTI using Jinja2 …
Remote Command Execution"): opened. Rare in reviewed code, critical when present.

**Correctness.** Counterexamples (`/tmp/review-python-security/codei/`, run `review-python-security-code-injection-cx2`):

| Shape | Expected | Result |
|---|---|---|
| `render_template_string(f"<p>Hello {name}</p>")` with `name` from `request.args` (`app/flaskapp.py:11`) | reported | reported |
| `env.from_string(tpl)` with `tpl` a Flask route parameter and `env = Environment()` (`app/flaskapp.py:16`) | reported | reported |
| `safe_eval(request.GET["expr"])` whose body is `eval(expr, {"__builtins__": {}}, {})` (`app/helpers.py:2`) | reported | reported, because the argument reads only `request`; with `expr = request.GET["expr"]; safe_eval(expr)` the validator-name rule clears it (the mechanism shown in command-injection) |
| `yaml.load(request.body, yaml.Loader)` (positional loader) | reported | reported |
| `yaml.load(request.body, Loader=yaml.CSafeLoader)` | silent | silent |
| Django `Template(request.POST["tpl"])` (`app/views.py:21`) | reported, with Django-specific advice | reported with "so a client can run code": overstated, see Text |
| `pickle.loads(bytes.fromhex(request.COOKIES["state"]))` (`app/views.py:44`) | reported | **missed**: `bytes.fromhex` is a library call outside the pass list, so the taint stops; `zlib.decompress`, `gzip.decompress`, `binascii.unhexlify`, `codecs.decode` do the same, and compressed pickles in cookies or bodies are the usual shape |
| `torch.load(request.FILES["model"])`, `numpy.load(…, allow_pickle=True)` | reported | missed (documented; the `python` pack's `bandit-b614-pytorch-load` flags every `torch.load`) |
| `pickle.loads(request.session.get("state"))` | silent (server-held data) | silent |

**Corpus.** 0 matches on every repository it ran on (section 11). No precision figure. The
sinks it must stay quiet on did stay quiet in the repositories that ran (paperless-ngx's own classifier pickle and
its sandboxed `from_string`).

**Overlap.** The `python` pack ports Bandit B301 (pickle), B302 (marshal), B307 (eval), B102 (exec), B614
(torch.load), B702 (mako); they flag every call whatever the source. Ruff S301/S506/S307 likewise; pylint W0123
(eval-used) and W0122 (exec-used). Nothing in Ruff, Bandit or pylint covers template injection (S701/B701 are about
autoescape). This rule adds the request source (it stays silent on the server's own pickles that B301 flags) and the
template sinks; that is real value.

**Text.** Description (154) fine. `TemplateSource`'s "so a client can run code" is true for Jinja2 and Mako but
not for a Django `Template`: the Django template language cannot call arbitrary code; a client template there reads
whatever the context and its objects expose (settings, related objects). Give Django templates their own message
("…a client can read any value the context reaches"). `Interpreter`'s "leave it only if the value is not client
data" is a no-op exemption: the rule only fires on client data; replace with "leave it only for a deliberate,
admin-only scripting feature".

**Verdict: rework**, `steer`, on. Changes: (1) carry the taint through byte-level decoders (`bytes.fromhex`,
`binascii.*`, `codecs.decode`, `zlib.decompress`, `gzip.decompress`, `bz2.decompress`, `lzma.decompress`) at least
for the deserializer family; (2) a separate message for Django `Template`/`Engine.from_string`; (3) the `Interpreter`
message edit.

## 5. `python-security-jwt-unverified`

**Value.** An unverified JWT accepted as identity is an authentication bypass. Sources hold up: GHSA-r9gp-7f88-9r54
(Lemur "JWT verifier honors attacker-supplied alg, enabling ATO"), GHSA-mcgx-2gcr-p3hp ("LTI JupyterHub
Authenticator does not properly validate JWT Signature"): opened; the PyJWT usage page says reading claims without
validation is ill-advised. Frequency: low, but every Python codebase that adopts SSO or third-party tokens writes
an unverified peek somewhere, so the rule's job is to tell the peek from the trust.

**Correctness.** The trust ranking (authentication hook, user lookup, identity assignment, returned to nobody) is a
reasonable proxy. Counterexamples (`/tmp/review-python-security/jwt/`, run `review-python-security-jwt-unverified-cx`):

| Shape | Expected | Result |
|---|---|---|
| `get_current_user` peeks `iss`/`kid` unverified, then verifies `jwt.decode(creds.credentials, key, algorithms=["RS256"])`; the peek used `token = creds.credentials` (`app/auth.py:16`) | silent: verified in the same function | **reported**: re-verification is matched by the token's text, so `token` vs `creds.credentials` breaks it |
| DRF `authenticate` peeks `iss`, returns `None` for foreign issuers, else `super().authenticate(request)` which verifies (`app/auth.py:25`) | silent | **reported**: the routing read counts as "authenticates the caller" |
| PyJWT 1 `jwt.decode(token, PUBLIC_KEY)` without `algorithms` | reported on PyJWT < 2 | missed (PyJWT 2 raises; the rule cannot tell the version) |
| `algorithms=["HS256", "RS256"]` with a public key | reported | missed (documented) |
| `repo.get(claims["sub"])` outside a hook (`app/auth.py:40`) | reported | reported: the README's gap ("a lookup through a callee whose names say nothing about users (`repo.get(...)`) is missed") is wrong, the `repo` receiver matches |
| DRF `authenticate` decodes unverified and returns `User.objects.get(pk=payload["user_id"])` (`app/auth.py:51`) | reported | reported |

**Corpus.** 4 matches in 2 repositories, all 4 read, 4 real:

- Netflix/lemur `lemur/auth/views.py:132`: `jwt.decode(id_token, secret, algorithms=[algo])` with `algo =
  header_data["alg"]` from `fetch_token_header` (called at line 118, a function of the run that reads the header unverified). This is GHSA-r9gp-7f88-9r54's code,
  still present at the pinned commit: the rule finds the advisory it cites. Real.
- Netflix/dispatch `src/dispatch/plugins/dispatch_core/plugin.py:92`, `:140`, `:142`: python-jose `jwt.decode`
  without `algorithms`. Real as a missing pin; the keys (an HMAC secret at 92, RSA JWKs at 140/142) make jose's own
  key-type checks refuse a `none` or HS/RS swap, so the message's "a token can choose an unsigned or key-confused
  check" overstates these three.

Silent where it should be: label-studio's `token_format.py:16` format sniff. **The rule could not finish on five
repositories**: open-webui (run alone, `review-python-security-jwt-unverified-open-webui__open-webui`), freqtrade,
pretix, netbox and addons-server (`review-python-security-jwt-<repo>`, the rule alone) all end with "content store
hard limit rejects new payloads", after 7 to 15 minutes. On freqtrade and addons-server the six other rules together
finished; on open-webui each finished alone. The writer's own jwt runs on open-webui and freqtrade have no `findings.tsv` either.
Not run on DefectDojo/django-DefectDojo, saleor/saleor, polarsource/polar, apache/superset, langgenius/dify, UB-Mannheim/escriptorium, andreynovikov/django-shop, lookit/lookit-api, TandoorRecipes/recipes, babybuddy/babybuddy: the extra Django repositories were run for the other rules only, and the large ones were dropped from the jwt queue after it failed on pretix, netbox and addons-server. A rule that cannot complete on a 330-file FastAPI app cannot ship on.

**Overlap.** None in Ruff, Bandit, pylint or mypy. Semgrep's `unverified-jwt-decode` flags every
`verify_signature: False` including format sniffs. This rule adds the trust reading and the header-`alg` flow.

**Text.** Description (157) fine. `Unverified`'s "leave it if they only pick the key" is the one exemption a team
needs, but the two false positives above are exactly that shape; the message should also cover "or when the same
token is verified before its claims are used". The README's jwt corpus line is the literal text
`CORPUS_PLACEHOLDER`: the writer never measured it.

**Verdict: rework**, `steer`, on once it completes on the corpus. Changes: (1) find and fix the
relation that exhausts the content store (open-webui, freqtrade, pretix, netbox, addons-server); the rule only needs
decode calls and the functions around them, so the include of the whole taint library and the `ju_in_args`/`ju_val` products over
every call are the first suspects; (2) treat the token as re-verified when any decode with the signature on, or a
`super().authenticate(request)`, runs in the same function, not only one whose token argument has the same text;
(3) report PyJWT `jwt.decode(token, key)` without `algorithms` when the project pins PyJWT below 2 (`pyproject`,
`requirements`), and a mixed HS*/RS*/ES* list; (4) word `OpenAlgorithms` as a missing pin ("Pass `algorithms=[...]`
naming only what your key signs with; python-jose otherwise trusts the token's `alg`") without promising an
exploit; (5) fix the README placeholder and its wrong `repo.get` gap.

## 6. `python-security-orm-lookup-injection`

**Value.** The client choosing lookup keys (`filter(**request.GET.dict())`) leaks hidden fields through related
lookups (`user__password__startswith`), and, on unpatched Django, is SQL injection (CVE-2025-64459 `_connector`,
CVE-2021-35042 `order_by`, CVE-2024-42005 `values()`, CVE-2022-34265 `Trunc`/`Extract`: all confirmed on Django's
security archive). GHSA-6hjj-gq77-j4qw (Label Studio "ORM Leak Vulnerability in Filtering Task") is the real-world
case. On current Django, `order_by(client_string)` is limited to fields and relations, so its worst case is an
ordering oracle on a related secret, which DRF's OrderingFilter docs warn about. Value: high for `**` splats,
medium for field names.

**Correctness.** Counterexamples (`/tmp/review-python-security/orm/`, run `review-python-security-orm-lookup-injection-cx`):

| Shape | Expected | Result |
|---|---|---|
| `apply_sort(Book.objects.all(), request.GET["sort"])`, helper `items.order_by(sort)` (`catalog/helpers.py:2`) | reported | **missed**: a receiver named `items`, `objects`, `books` passed in as a parameter is not a queryset |
| `order_by(p.validated_data["ordering"])` with `ordering = serializers.CharField()` (`catalog/views.py:22`) | reported: a free-text field | **missed**: `validated_data` is trusted wholesale, though only its *keys* are declared, not its values |
| `values(*p.validated_data["fields"])` with a `ListField(child=CharField())` (`catalog/views.py:28`) | reported | **missed** (same) |
| `for k in request.GET: if k not in ALLOWED: raise` then `filter(**request.GET.dict())` (`catalog/views.py:35`) | silent | **reported** (documented false positive) |
| `sort.lstrip("-") not in {"title"}` check then `order_by(sort)` | silent | silent |
| `filter(Q(**request.GET.dict()))`, `values(*request.GET.getlist("f"))` | reported | reported |
| Django URL kwarg in `order_by(sort)` | reported | missed (documented) |

The `validated_data` trust is right for the `LookupKeys` clause (a serializer's validated dict has declared keys)
and wrong for the `FieldName` clause, which takes a *value* as a field name.

**Corpus.** 3 matches in 2 of the Django repositories in section 11, all read, 2 real (67%):

- lookit/lookit-api `exp/views/user.py:54`: the researcher-facing participant list does
  `qs.order_by(self.get_ordering())` with `get_ordering()` returning `request.GET["sort"]`; `qs` is a user
  queryset, so `?sort=password` orders participants by password hash. Real: the DRF docs' exact warning.
- lookit/lookit-api `studies/queries.py:585`: `_apply_sort(queryset, sort)` with `sort` from the study list's
  `request.GET` in `exp/views/study.py:290`, through `dict(self.request.GET)`: cross-file, any related field. Real.
- andreynovikov/django-shop `shop/admin/product.py:534`: `Stock.objects…order_by(request.GET.get("o"))` in an admin
  view that raises `PermissionDenied` unless `request.user.is_staff`. Staff already sort the admin changelists;
  not a leak a team would act on. Not real.

escriptorium's ordering (`app/apps/api/views.py:528`, `:530`, reported in the writer's earlier run) is now silent:
the `if order_field == …`/`elif order_field in (…)` guard is recognised. Not run (queue cut or engine limit): pretix/pretix, DefectDojo/django-DefectDojo, saleor/saleor.

**Overlap.** None in Ruff, Bandit, pylint, flake8-django or the drf pack. DRF's own `OrderingFilter` with
`ordering_fields` is the framework fix the message should name.

**Text.** Description (130) is clear. `LookupKeys`' "Not needed when every key is fixed in code." is dead text (fixed
keys are never reported); say "Leave it if the keys were checked against an allow-list in another function."
`FieldName` should name DRF's `OrderingFilter`/`ordering_fields` for DRF views. The README's corpus line is the
literal text `CORPUS`.

**Verdict: rework**, `steer`, on after change (3); until then `log`. Changes: (1) `FieldName`: stop trusting
`validated_data`/`cleaned_data` values (keep trusting them only as the keys of a `**` splat); (2) accept any
receiver that a caller passes a queryset to (a parameter given `X.objects…` or a name bound to one), not only
`qs`-like names; (3) leave alone views guarded for staff (`is_staff`/`is_superuser` checks that raise,
`staff_member_required`, `IsAdminUser`, `ModelAdmin` methods); (4) recognise the key loop
`for k in request.GET: if k not in ALLOWED: raise`; (5) message and README edits above. Precision on 3 reads
is thin; with (3) all read matches are real.

## 7. `python-security-path-traversal`

**Value.** Arbitrary file read, write or delete. Sources hold up: GHSA-9jj8-v89v-xjvw (changedetection.io
"unauthenticated static path traversal"), GHSA-ph9w-r52h-28p7 (langflow profile-picture file read): opened.
Frequency: moderate in small FastAPI apps, rare in the mature corpus.

**Correctness.** The sanitiser set (basename, `secure_filename`, resolve-and-contain with a separator) matches the
OWASP recipe, and the rule correctly refuses `startswith(base)` without a separator. Counterexamples
(`/tmp/review-python-security/path/`, run `review-python-security-path-traversal-cx2`):

| Shape | Expected | Result |
|---|---|---|
| FastAPI path parameter `{name}` in `os.path.join(BASE, f"{name}.json")` (`api/routes.py:15`) | silent: `[^/]+` plus a fixed suffix cannot leave `BASE` | **reported** (false positive) |
| `ext = file.filename.rsplit(".", 1)[-1]` then `open(join(BASE, f"{uuid4().hex}.{ext}"), "wb")` (`api/routes.py:21`) | silent: only the extension, after a server name | **reported** (false positive) |
| `Image.open(join(BASE, path))`, `pd.read_csv(join(BASE, path))` with a client `path` | reported | **missed**: PIL, pandas, `zipfile.ZipFile`, `tarfile.open` are not sinks |
| `os.path.exists(join(MEDIA, request.GET["n"]))` | reported (existence oracle) | missed |
| `if ".." in name: raise` then `open(join(MEDIA, name))` (`docs/views.py:16`) | reported: an absolute path still escapes | reported |
| Django `<path:relpath>` URL kwarg in `open(join(MEDIA, relpath))` | reported | missed (documented: URL kwargs are not sources; `path:` is the converter that allows `/`) |
| FastAPI `path: str` in `FileResponse(join(BASE, path))` (`api/routes.py:37`) | reported | reported |

**Corpus.** 12 matches, all in rachelos/we-mp-rss; every other web repository that ran had none
(section 11). All 12 read, 7 real (58%):

- Real (7): `apis/tools.py:181` (`os.remove` of `./data/docs/{mp_id}/{zip_filename}`, both from the JSON body); the
  same `mp_id` directory reaching `core/exporter.py:223` (`makedirs`), `:250`, `:291` (`open(…, "w")`),
  `tools/mdtools/html2doc.py:206` and `tools/mdtools/pdf_extractor.py:104`, `:206` (writes): an authenticated user
  writes and deletes outside `./data/docs`.
- Not real (5):
  - `apis/tools.py:752`: `output_format` is only the extension after a uuid name, and PIL has already accepted it as
    an image format (my "extension only" counterexample);
  - `apis/tools.py:793`, `:807`: path parameter `filename` joined to `temp_dir`, then `abspath` and
    `startswith(temp_dir)` without a separator. A path parameter cannot hold `/`, so the only climb is `..`, which
    the check stops; the missing separator only matters for a `/`-bearing value;
  - `apis/user.py:566`: `UploadFile.filename` appended after `"{username}_{date}_"` in the same path segment, so a
    `../` in it needs a directory named `alice_20260101_..` to exist;
  - `core/rss.py:46`: path parameter `content_id` with `.json` appended after `normpath` and a `startswith` check.

The writer counted 11 of these 12 as real "when a weak check counts as real". I count what a reviewer would ask a
team to change: the five above are safe as written.

**Overlap.** None in Ruff, Bandit, pylint. The `python` pack's `ruff-pth*` rules are pathlib style only.

**Text.** Description (159) is good. Message (199, at the limit) is good and gives the exemption.

**Verdict: rework**, `log`, on (58% is below the 90% bar for steer). Changes: (1) a FastAPI/Starlette or Flask
path parameter (`{name}`, `<name>`, never `{name:path}`/`<path:name>`) that is concatenated with a fixed suffix or
prefix inside one path segment cannot climb and is not reported; (2) a value reduced to its extension
(`rsplit(".", 1)[-1]`, `splitext(...)[1]`) and appended after a server-generated name is not reported; (3) add
read sinks that leak files: `PIL.Image.open`, `pandas.read_*`, `zipfile.ZipFile`, `tarfile.open`, `os.path.exists`
/`isfile` (oracle), `os.walk`, `Path.iterdir`; (4) make Django URL kwargs captured by a `<path:…>` or `<str:…>`
converter sources (section 9). With (1) and (2), all 12 read matches would have been real.

## 8. `python-security-ssrf`

**Value.** Server-side request forgery into internal services and cloud metadata. Sources hold up:
GHSA-p59w-9gqw-wj8r (Label Studio "SSRF on Import Bypassing SSRF_PROTECTION_ENABLED"), GHSA-h6x2-583h-x99r (Open WebUI
"DNS Rebinding SSRF Bypass"): opened. Both advisories are about checks that *exist* and are bypassed, which matters
for the message below.

**Correctness.** The "does the client control the host" reading (`ss_slot`, `ss_open`, the `%`/`format` placeholder
scan) is precise and is the rule's best part. Counterexamples (`/tmp/review-python-security/ssrf/`, run
`review-python-security-ssrf-cx`):

| Shape | Expected | Result |
|---|---|---|
| `URLValidator(schemes=[…])` then `requests.get(url)` (`app/views.py:15`) | reported: format check only | **missed**: `validate(url)` has a validator name |
| `TypeAdapter(HttpUrl).validate_python(url)` then `requests.get(url)` (`app/views.py:21`) | reported: the README itself says `HttpUrl` is not a sanitiser | **missed**: `validate_python` has a validator name |
| `if url in BLOCKED: raise` (`app/views.py:28`) | reported | missed (documented) |
| `get_client().get(request.GET["url"])` with a factory returning `requests.Session()` (`app/views.py:32`) | reported | missed (documented) |
| `requests.get(API + request.GET["path"])` with `API = "https://api.example.com"` (`app/views.py:36`) | reported (`@evil.com` moves the host) | reported: better than the README's gap says |
| `f"{API}/items/{request.GET['id']}"` | silent | silent |
| `requests.get(request.GET["url"])` (`app/views.py:44`) | reported | reported |

**Corpus.** 10 matches in 5 repositories, all read, 2 real (20%):

| Match | Real? | Why |
|---|---|---|
| we-mp-rss `apis/tools.py:843` | real | `/image/proxy` route with no auth dependency fetches any `url` query value, `follow_redirects=True` |
| we-mp-rss `apis/tools.py:708` | real | authenticated image-crop route downloads `image_url` from the form |
| we-mp-rss `apis/res.py:67` | no | `parsed_url = urlparse(path); host = parsed_url.netloc; if host not in hosts: return …`: a correct allow-list two assignments away from `path`; `ss_checked` follows one |
| open-webui `routers/functions.py:129`, `routers/tools.py:494`, `routers/openai.py:1061`, `:1087` | no (4) | every route takes `user=Depends(get_admin_user)`; the first two carry the comment "This is NOT a SSRF vulnerability: This endpoint is admin-only", and an Open WebUI admin can already run arbitrary Python tools |
| linkding `bookmarks/services/http_client.py:294` | no | `session.mount("http://", GuardedHTTPAdapter())`: a connection-time private-address block, the right fix, mounted three lines above the call |
| mealie `mealie/pkgs/safehttp/fetch.py:186` | no | `AsyncClient(transport=AsyncSafeTransport(...))` in a package named `safehttp` that blocks private, loopback and link-local targets |
| dispatch `plugins/dispatch_core/plugin.py:188` | no | `f"https://public-keys.auth.elb.{region}.amazonaws.com/{kid}"`: the client's `kid` is after the host's `/`; `region` comes from settings. A rule bug, below |

**Rule bug (confirmed).** `ss_slot`'s second clause reuses the variable `j` inside the `count` aggregate and in the
literal after it (`child(s, q, j), j < k, ss_piece_scheme(q)`), so the count only looks at the scheme piece itself.
Any f-string whose scheme piece does not end the host (`"https://"`, `"https://keys."`) treats every later
interpolation as host-controlling. `/tmp/review-python-security/ssrf3/` (run `review-python-security-ssrf-cx3`)
shows it on the commonest shape there is: `requests.get(f"https://{settings.API_HOST}/items/{request.GET['id']}")`
is reported; `f"https://api.example.com/items/{…}"` is not. `/tmp/review-python-security/ssrf2/` reproduces
dispatch's line. The cases never mix a host interpolation with a later path interpolation, so the tests pass.

The repositories of 1,000 files or more where it finished have no match: pretix/pretix, mozilla/addons-server, DefectDojo/django-DefectDojo, saleor/saleor, polarsource/polar, apache/superset, netbox-community/netbox. Not run: langgenius/dify.

**Overlap.** Ruff S310 / Bandit B310 (ported in the `python` pack) audit only `urlopen`'s scheme, from any source.
Nothing covers requests/httpx/aiohttp. The rule adds the whole check.

**Text.** Description (121) good. The message's fix, "Check the host against an allow-list", is wrong for the two
shapes the corpus actually has: a product whose feature is fetching user URLs (link preview, import, webhooks) cannot
allow-list, and the right fix there is a connection-time block of private and link-local addresses with redirects
re-checked (linkding already does this). Suggested: "…call internal services. Allow-list the host, or block private
and link-local addresses at connection time (redirects included); leave it if a guarded transport is mounted."

**Verdict: rework**, `log`, on (20% read precision; keep it out of steer until the fixes are measured).
Changes: (1) fix `ss_slot` (use a fresh variable for the scheme piece's index); (2) treat a client whose function
mounts an adapter (`session.mount(…, X())`) or passes `transport=` to `httpx.Client`/`AsyncClient`, or a call
through a module the run names `safehttp`/`ssrf`/`guarded`, as guarded; (3) leave alone routes whose dependencies
or decorators require an admin (`Depends(get_admin_user)`, `is_superuser`, `IsAdminUser`, `staff_member_required`);
(4) follow a check through a chain of assignments (`path → parsed_url → host`), not one; (5) do not count
format-only validators as checks: Django `URLValidator`, `validators.url`, pydantic `HttpUrl`/`TypeAdapter`, whose
names contain `valid`; (6) the message edit above. On these 10 reads, (1) to (4) leave the 2 real matches and
remove the 8 others.

## 9. The shared library, `rules/lib/security.rerail`

Every rule's precision and recall comes from this file, so its defects are the pack's.

1. **Validator names by substring.** `sec_validator_name` accepts any function or call name containing `safe`,
   `valid`, `allow`, `check`, `verif`, `sanitiz`, `whitelist`, `trusted`, or starting with `clean`. A name passed to
   such a call is "checked" for its whole function (`sec_checked`), and such a function's return carries no taint.
   Shown: `host = request.GET["host"]; git.run_healthcheck(host)` hides a `shell=True` ping (command-injection cx);
   `URLValidator()`'s `validate(url)` and `TypeAdapter(HttpUrl).validate_python(url)` clear an SSRF (ssrf cx), though
   the README itself says format checks are no sanitiser. Fix: match a *prefix* verb (`is_`, `validate`, `check_`,
   `ensure_`, `sanitize`, `clean`), never count a function that itself reaches the rule's sink, and keep a deny list
   of known format-only validators.
2. **Validated data stays tainted regardless of type.** `cleaned_data`/`validated_data` inherit the taint of the
   form or serializer's input, whatever the field type; FastAPI `Query(pattern=…)` and pydantic `Literal`/`Enum`
   fields stay tainted too. That is right for a `CharField` and wrong for an `IntegerField` or a `ChoiceField`.
   Five of the false positives above (sql-injection x4, command-injection x1) are this. Fix: read the declared field
   type (serializer and form class attributes, pydantic annotations, `Query`/`Path` constraints) and stop the taint
   at numbers, UUIDs, dates, booleans, enums, literals and choices.
3. **Django URL kwargs are not sources.** `def view(request, slug)` and `self.kwargs[...]` carry whatever the
   URLconf captured, including `<str:…>` and `<path:…>` text with `/`. The README calls them "mostly numbers"; the
   converter says which. This is the largest recall gap for a Django team (every rule missed it in the cx).
   Fix: read `path()`/`re_path()` converters and taint `str`, `slug`, `path` captures.
4. **Library calls end the taint.** Good for precision (`int()`, `basename`), but it also ends at byte decoders
   (`bytes.fromhex`, `zlib.decompress`, `codecs.decode`), so `pickle.loads(zlib.decompress(request.body))` is
   silent. A short pass list per sink family would fix code-injection's miss.
5. **Context-insensitive helpers.** A helper's parameter is tainted for every call when one caller passes request
   data, and its return is tainted at every call that passes *any* tainted argument. No false positive came from
   this in the corpus; it is the next one to expect in large codebases with shared utilities.
6. **Cost.** jwt-unverified, which uses only a slice of the library, exhausts the engine's content store on
   open-webui, freqtrade, pretix, netbox and addons-server (section 5); the six other rules finished on all three of
   open-webui (one by one), freqtrade and addons-server (together). The taint rules run together also hit the limit on pretix and netbox,
   so a team with a 1,000-file repository may not get this pack to complete in one run.

## 10. Pack-level findings

**Rule order for a reader.** `pack.json` is alphabetical. Read: sql-injection, command-injection, code-injection
(the critical injection trio, silent in mature code), jwt-unverified, path-traversal, ssrf, orm-lookup-injection.

**Missing rules.** The lead's own build list (`NOTES-python-security.md`, section 3) has twelve rules; five never
landed, and they are the ones a Django or Flask team meets most often:

- `open-redirect` (`redirect(request.GET["next"])` without `url_has_allowed_host_and_scheme`): GHSA-vqv8-j3mj-wjxj
  (wger), GHSA-564j-v29w-rqr6 (Khoj); the lead's probe counted 17 request-fed redirects in the corpus, all checked
  (the lead's figure, not re-measured). A team-checklist item for every login flow.
- `xss-unescaped-html` (`mark_safe`/`format_html` format string/`Markup`/`HTMLResponse` with request data). Bandit
  B308/B703/B704 flag every `mark_safe`, so a request-sourced version is exactly the "context Bandit lacks" this
  pack exists for.
- `csrf-exempt-session-write`, `timing-unsafe-compare`: no linter covers either; both need the cross-function reading
  this engine has.
- `cors-credentials-any-origin`: covered for Starlette/FastAPI only, by `fastapi-practice-cors-wildcard-with-credentials`
  in the fastapi pack; the django-cors-headers and flask-cors shapes the notes list are covered nowhere.

Also: the notes moved `mass-assignment` to the drf pack (`input.mass-assignment`), and the drf pack has no such
rule. Someone should own it.

**Rules in the wrong pack.** None needs moving. `orm-lookup-injection` is Django-only but security-first; it fits
here. `sql-injection` and `sqlalchemy-practice-text-interpolation` both fire on request-built `text()`; keep both (one
is request-aware and steers, the other is a log-mode hygiene rule) and cross-reference them in both READMEs.

**README quality.** Two corpus lines are unfilled placeholders (`CORPUS_PLACEHOLDER` under jwt-unverified, `CORPUS`
under orm-lookup-injection); the ssrf section is the header comment pasted without Markdown (code examples outside
a fence, no corpus line at all); the jwt gap about `repo.get` is wrong (the rule catches it). The pack validation
contract passes (`npx vitest run test/engine/catalog/packs.test.ts -t "pack python-security"`: 16 passed), so none
of this is caught by a check.

**Can it be pitched on its own?** Not yet. On the 29 web repositories that ran the three critical rules are silent (good for
trust, but a team sees nothing), and the two rules that do fire are at 58% and 20% read precision, with one
confirmed logic bug. Its strongest result is jwt-unverified finding Lemur's published advisory at its pinned commit.
After the reworks above, and with open-redirect and XSS added, it is a credible "framework-aware security" pack
next to Bandit; today it is a set of insurance rules a team would install alongside the django or fastapi pack, not
a reason to install Rerail.

## 11. Corpus repositories and commits

The 20 repositories of the brief plus 10 more web repositories from `MANIFEST.tsv` (no new clone). "brief" rows are
BRIEF.md's corpus. Cells: matches; `fail` = the engine stopped with "content store hard limit rejects new payloads";
`not run` = not reached before the report (queue cut); `n/a` = not a Django project (orm-lookup-injection reads
Django ORM calls only). Commits are the pinned ones in `MANIFEST.tsv`.

Why the large repositories have gaps: the machine was shared with the other reviewers (load average 40 to 80), a
run of a repository over 1,000 files waited 20 to 80 minutes for one of three slots, and the grouped six-rule run hit
the content store limit on pretix and netbox. I ran the rules most likely to match (ssrf, path-traversal,
orm-lookup-injection) one at a time on the large repositories first, and stopped the queue when the review closed:
the `not run` cells for sql-injection, command-injection and code-injection on pretix, netbox, DefectDojo, saleor,
polar, superset and dify are that queue, not a rule failure. jwt-unverified was not retried on the large
repositories after it failed alone on pretix, netbox and addons-server.

| Repository | Commit | .py | Stack set | sql | cmd | code | jwt | path | ssrf | orm |
|---|---|--:|---|--:|--:|--:|--:|--:|--:|--:|
| sissbruecker/linkding | `27b7303baf41` | 229 | brief | 0 | 0 | 0 | 0 | 0 | 1 | 0 |
| healthchecks/healthchecks | `c1039ad23395` | 658 | brief | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| paperless-ngx/paperless-ngx | `faa6f09f289b` | 424 | brief | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| HumanSignal/label-studio | `2f42caecbf6c` | 775 | brief | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| makeplane/plane | `723bb178f97b` | 784 | brief | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| inventree/InvenTree | `939df605f43d` | 731 | brief | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| pretix/pretix | `40ea9097d0b2` | 1228 | brief | not run | not run | not run | fail | 0 | 0 | not run |
| mozilla/addons-server | `7e3374fe5383` | 1426 | brief | 0 | 0 | 0 | fail | 0 | 0 | 0 |
| DefectDojo/django-DefectDojo | `cd0a4483a224` | 2601 | brief | not run | not run | not run | not run | 0 | 0 | not run |
| saleor/saleor | `e1ccf4d58002` | 4385 | brief | not run | not run | not run | not run | 0 | 0 | not run |
| fastapi-users/fastapi-users | `e9c57ff7ff2f` | 88 | brief | 0 | 0 | 0 | 0 | 0 | 0 | n/a |
| fastapi/full-stack-fastapi-template | `77e550a73bb4` | 43 | brief | 0 | 0 | 0 | 0 | 0 | 0 | n/a |
| jupyterhub/jupyterhub | `3ca37442a18e` | 158 | brief | 0 | 0 | 0 | 0 | 0 | 0 | n/a |
| open-webui/open-webui | `0666e59d26a4` | 330 | brief | 0 | 0 | 0 | fail | 0 | 4 | n/a |
| freqtrade/freqtrade | `7fcb509a434c` | 487 | brief | 0 | 0 | 0 | fail | 0 | 0 | n/a |
| Netflix/dispatch | `dd2837e82a0b` | 717 | brief | 0 | 0 | 0 | 3 | 0 | 1 | n/a |
| mealie-recipes/mealie | `8dbb92808d5a` | 723 | brief | 0 | 0 | 0 | 0 | 0 | 1 | n/a |
| polarsource/polar | `872272fbfc6e` | 2522 | brief | not run | not run | not run | not run | not run | 0 | n/a |
| apache/superset | `fdf61721d056` | 3191 | brief | not run | not run | not run | not run | 0 | 0 | n/a |
| langgenius/dify | `724ac15121ee` | 5191 | brief | not run | not run | not run | not run | not run | not run | n/a |
| rachelos/we-mp-rss | `8fe5bf938743` | 221 | extra | 0 | 0 | 0 | 0 | 12 | 3 | n/a |
| Netflix/lemur | `3a3c2c201d42` | 337 | extra | 0 | 0 | 0 | 1 | 0 | 0 | n/a |
| wger-project/wger | `f903c66d3c0c` | 766 | extra | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| netbox-community/netbox | `69978099811c` | 1299 | extra | not run | not run | not run | fail | 0 | 0 | 0 |
| taigaio/taiga-back | `8ab6a4e3b5f6` | 933 | extra | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| UB-Mannheim/escriptorium | `7e06bd31732c` | 260 | extra | 0 | 0 | 0 | not run | 0 | 0 | 0 |
| andreynovikov/django-shop | `2b8dc600166b` | 512 | extra | 0 | 0 | 0 | not run | 0 | 0 | 1 |
| lookit/lookit-api | `78562b5039ae` | 329 | extra | 0 | 0 | 0 | not run | 0 | 0 | 2 |
| TandoorRecipes/recipes | `22b5841a7828` | 418 | extra | 0 | 0 | 0 | not run | 0 | 0 | 0 |
| babybuddy/babybuddy | `1638cfb02e26` | 241 | extra | 0 | 0 | 0 | not run | 0 | 0 | 0 |

## 12. Commands run

All from the worktree root, at `nice -n 19` (the corpus script sets it for the engine).

- Each rule's own tests: `node dist/cli/cli.js rules test packs/python-security/rules/<rule>.rerail`: 7 of 7 pass.
- Pack contract: `npx vitest run test/engine/catalog/packs.test.ts -t "pack python-security"`: 16 passed.
- Counterexamples: `~/rerail-bench/python-arch/bin/corpus-run /tmp/review-python-security/<dir>
  review-python-security-<rule>-cx packs/python-security/rules/<rule>.rerail` for `sqli`, `cmdi`, `jwt`, `orm`,
  `ssrf` (`-cx`), `codei`, `path` (`-cx2`: the first attempt was queued past its shell timeout), and `ssrf2`, `ssrf3`
  (`review-python-security-ssrf-cx2`, `-cx3`) for the f-string bug. Outputs: `~/rerail-bench/python-arch/runs/<name>/`.
- Corpus, three kinds of runs (the content store limit forced the split):
  - `review-python-security-all-<repo>`: all seven rules together on linkding, fastapi-users, fastapi template,
    jupyterhub, healthchecks, paperless-ngx (finished); open-webui and freqtrade failed (content store), and four more
    were stopped by the reviewer and rerun below;
  - `review-python-security-six-<repo>` (the six non-jwt rules, orm only on Django) and
    `review-python-security-jwt-<repo>` (jwt alone);
  - `review-python-security-<rule>-<repo>`: one rule alone, for open-webui and the repositories of 1,000 files or
    more after the grouped runs hit the content store limit on pretix and netbox.
- Text searches of the corpus (`rg`) for `verify_signature: False` to find the JWT shapes the rule should stay quiet
  on (saleor `is_saleor_token`, addons-server `jwt_decode_handler`, label-studio `token_format`, open-webui
  `oauth.py`).
- Sources: the GitHub advisory API for the 16 GHSA ids the README and notes cite for these rules (all exist and match the
  description), and Django's security archive for CVE-2021-35042, CVE-2022-34265, CVE-2024-42005, CVE-2025-57833,
  CVE-2025-64459, CVE-2026-1312.

## Applied

What the fix changed, per finding (the pack's rules, cases, `pack.json`, README and `rules/lib/security.rerail`;
revision 1 kept, no rule cut, no id changed).

**Shared library (`rules/lib/security.rerail`, section 9).**
1. Validator names: a check is now a call whose name *starts* with a checking verb (`is_`, `validate`, `valid_`,
   `check_`, `ensure_`, `verify_`, `sanitize`/`sanitise`, `clean`, also after `_`) or Django's
   `url_has_allowed_host_and_scheme`; never a substring. A call of a function of the run that reaches the including
   rule's sink (each rule declares its sinks in the extensible `sec_sink_fn`) is never a check, and such a function's
   return is not trusted. Format-only validators (`URLValidator`, `validators.url`, pydantic `validate_python`/`json`)
   are not checks. `if x in S: raise` (or `abort()`) is a block list and no longer a check.
2. Typed validated values (`sec_typed`): form and serializer fields declared as numbers, booleans, UUIDs, dates or
   choices (on the class or a base of the run, also through `self.get_serializer()` and `serializer_class`), pydantic
   fields typed `Literal`, an `Enum`, `int`, `UUID`, dates or `Field(pattern=<fixed words>)`, and FastAPI parameters
   with `Query/Path(pattern=…)` of fixed words or digits stop the taint inside `sec_step`.
3. Django URL kwargs are sources: a view function's parameter captured by `path()`/`re_path()`/`url()` with a `str`,
   `slug`, `path` or no converter, or a non-digit regex group; `self.kwargs` in a class-based view routed with a text
   capture. `sec_segment_param` tells one-segment captures from `path:`.
4. Byte decoders carry the taint (`base64`, `binascii`, `bytes.fromhex`, `codecs.decode`, zlib/gzip/bz2/lzma
   decompression, `io.BytesIO`/`StringIO`).
5. Aggregates: `sec_text_before`/`sec_text_after` give the literal text around a spliced value (f-strings, `+`
   chains, `%`, `.format()`, with the file's string constants resolved), so a rule judges only the part that decides
   its sink (ssrf: the host).
6. Admin-only code (`sec_admin_only`, `sec_admin_source`), shared by ssrf and orm-lookup-injection.
7. Not changed: helpers stay context-insensitive (no false positive measured from it).

**`python-security-sql-injection`** (steer, on). Typed values through the library (the four counterexample false
positives are near misses in `shop/typed.py`, `api/typed_routes.py`); asyncpg and `databases` sinks with handles
resolved through `asyncpg.connect/create_pool`, `acquire()`, `databases.Database` and annotations (`api/asyncdb.py`);
a new `SqlFragment` message for a whole client string in `text()`/`literal_column()` inside `order_by`/`where`/…,
`.extra()` and `RawSQL` (`api/fragments.py`; `session.execute(text(sql))` stays the SQL-lab exemption); pandas
`read_sql`; URL-kwarg cases (`shop/urls.py`, `shop/kwviews.py`); check semantics (`shop/checks.py`); description and
message rewritten. Corpus after the fix: 0 matches on 4 repositories.

**`python-security-command-injection`** (steer, on). The rule's own validator copy replaced by the library's checks
(`run_healthcheck`, `check_disk`, `checkout_branch` are reported inside the helper and at the later call; a block
list is reported); `ChoiceField`/`IntegerField` values silent (`netops/typed.py`); URL-kwarg case; messages say
"unless it was checked against a fixed allow-list first". Corpus after the fix: 0 matches on 4 repositories.

**`python-security-code-injection`** (steer, on). Decoders through the library (`app/decoders.py`: `bytes.fromhex`,
`zlib`, `gzip`+`base64`, `binascii`, `codecs`, `BytesIO`); `numpy.load(allow_pickle=True)` and
`torch.load(weights_only=False)` added; a separate `DjangoTemplateSource` message ("a client can read any value its
context reaches") for `django.template.Template` and Django `Engine.from_string` (`app/django_tpl.py`); `Interpreter`
exemption now "a deliberate, admin-only scripting feature"; `validate_expr`/`safe_eval` cases (`app/calc.py`);
description without code. Corpus after the fix: 0 matches on 4 repositories.

**`python-security-jwt-unverified`** (steer, on). Cost: no clause pairs two unbound nodes of one function any more
(the root cause, measured at 167 million rows on dispatch); the taint library is no longer included. It now finishes
on open-webui (46 s CPU, 798 s before) and freqtrade. Re-verification: any signed decode in the function,
`super()`/`Base.authenticate`, `get_validated_token`, or a verifying helper given the same token or its bound name.
PyJWT 1 without `algorithms` (from `encode().decode()` or a `setup.py` pin) and mixed HS/public-key lists are
reported. Messages rewritten (OpenAlgorithms as a missing pin); README placeholder and the wrong `repo.get` gap fixed.
Corpus: 4 of 4 real on 6 repositories; pretix, netbox and addons-server not re-run (local engine runs were stopped).

**`python-security-path-traversal`** (log, on). Each value carries what it can do in a path: a one-segment route
parameter with fixed text in its segment, and free text after fixed text of its segment, are no longer reported at
open/send/remove sinks (all five false positives the review read); PIL, pandas, zipfile, tarfile, `os.walk`, glob,
`iterdir`, `make_archive` sinks with file-like arguments excluded; a separate, weaker `PathProbe` message for
existence oracles; Django `<path:>`/`<str:>` cases. Corpus: 11 of 11 real (we-mp-rss), 0 on four more.

**`python-security-ssrf`** (log, on). The `ss_slot` bug is gone with the piece-by-piece logic: the host is judged
from `sec_text_before`; guarded clients (an httpx `transport=`/aiohttp `connector=` of a class of the run, built
directly or by a function of the run, or a mounted adapter class of the run); admin-only sources dropped; the check
chain follows only names derived from the URL's own text (`urlparse`, attributes, methods), not values computed with
it (a check of the fetched response once cleared the URL); format-only validators and block lists are no checks;
client factories followed; message offers the connection-time block. Corpus: 2 of 3 real on the runs, the third
(mealie's factory-built safe transport) fixed and covered by a case, not re-run.

**`python-security-orm-lookup-injection`** (steer, on). Free-text `validated_data`/`cleaned_data` values are field
names (typed fields cut); queryset receivers resolved through calls, returns, framework hooks and related managers
instead of names; staff-only code left alone through the library (django-shop's admin view now silent); the
request-key allow-list loop and set checks recognised; messages and header rewritten; URL-kwarg cases. Corpus: 2 of 2
real (lookit), 0 on three more.

**Pack level.** `pack.json` and the README list the rules in the review's reading order; the README has no
placeholder, the ssrf section is rewritten as Markdown, and every corpus line is this fix's re-measurement. Missing
rules (open-redirect, xss-unescaped-html, csrf-exempt-session-write, timing-unsafe-compare,
cors-credentials-any-origin for Django/Flask, an owner for mass-assignment) and the cross-reference in the
sqlalchemy pack's README are outside this pack's files and are left to the coordinator.
