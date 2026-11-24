# python-security: mining notes

What the pack is for: untrusted HTTP request data in Django, Django REST framework, FastAPI/Starlette and Flask code
that reaches SQL, a shell, an interpreter or deserializer, a file path, an outgoing URL, a redirect or raw HTML, in
the same function or through the project's own helpers in other files; and the framework-specific auth and transport
settings that undo a protection (unverified JWTs, credentialed CORS for any origin, CSRF exemption on session views,
timing-unsafe secret checks). Bandit's single-call checks are already in the `python` pack; a rule here earns its
place by following the data from the request (or reading the framework's configuration), which is what keeps it
quiet on the calls Bandit flags regardless of where the value came from.

The research below was done by reading the pages listed (two read-only research passes, each opening every page it
cites); their full notes, with every URL opened, are kept beside the writers' specs. Bandit and Ruff overlap was
checked by running `ruff check --select S` (0.17.6) and `bandit` (1.9.4) on a scratch file holding one vulnerable and
one safe line per shape (results in section 2), and against the `python` pack's ports in `packs/python/pack.json`
(50 Bandit rules, among them B301 pickle, B308 mark_safe, B606, B609, B610 extra; it ports no Ruff `S` rule).

## 1. Sources read

### Framework and language documentation

| Source | URL | What it contributed |
|---|---|---|
| Django: Performing raw SQL queries | https://docs.djangoproject.com/en/5.1/topics/db/sql/ | Pass values as `params`; "Do not use string formatting on raw queries or quote placeholders in your SQL strings" |
| Django QuerySet API (`extra()`, `order_by()`, lookups) | https://docs.djangoproject.com/en/5.1/ref/models/querysets/ | `extra()` is a last resort and `'%s'` quoted placeholders are unsafe; lookups are keyword names the ORM trusts |
| Django query expressions (`RawSQL`, `Func.template`) | https://docs.djangoproject.com/en/5.1/ref/models/expressions/ | `RawSQL(sql, params)`; `Func(**extra)` values are interpolated into SQL, never parameters |
| Django security overview | https://docs.djangoproject.com/en/5.2/topics/security/ | Querysets are parameterised; care with `extra()`, `RawSQL`, `mark_safe`, `is_safe` |
| Django security release archive | https://docs.djangoproject.com/en/dev/releases/security/ | Django's own SQL-injection CVEs through `**kwargs` and `order_by()` (CVE-2021-35042, CVE-2022-28346, CVE-2024-42005, CVE-2025-57833, CVE-2025-64459, CVE-2026-1312), redirect-validator CVEs (CVE-2015-2317, CVE-2017-7233, CVE-2018-14574) |
| Django security releases, 2021-07-01, 2024-08-06, 2025-09-03, 2025-11-05, 2026-02-03 | https://www.djangoproject.com/weblog/2021/jul/01/security-releases/ (and the other dates) | Each is an app passing a request-built dict or field name to the ORM |
| Django `django/utils/http.py` (url_has_allowed_host_and_scheme) | https://raw.githubusercontent.com/django/django/main/django/utils/http.py | The redirect check: host and scheme, backslash form, empty URL refused |
| Django auth views (`RedirectURLMixin`) | https://raw.githubusercontent.com/django/django/main/django/contrib/auth/views.py | The reference shape of a safe `next` redirect |
| Django shortcuts (`redirect`) | https://docs.djangoproject.com/en/5.2/topics/http/shortcuts/ | `redirect(to)` uses a URL "as-is" |
| Django utils (`format_html`, `mark_safe`) | https://docs.djangoproject.com/en/5.2/ref/utils/ | `format_html` escapes its arguments, never its format string; `mark_safe` marks without escaping; calling `format_html()` without args is deprecated (5.0) |
| Django request/response | https://docs.djangoproject.com/en/5.2/ref/request-response/ | `HttpResponse` defaults to `text/html`; `FileResponse` takes an open file with no containment |
| Django CSRF reference and how-to | https://docs.djangoproject.com/en/5.2/ref/csrf/, https://docs.djangoproject.com/en/5.2/howto/csrf/ | What `csrf_exempt` removes; safe methods; sanctioned exemption patterns |
| Django file storage and `get_valid_filename` | https://docs.djangoproject.com/en/5.1/ref/files/storage/, https://raw.githubusercontent.com/django/django/main/django/utils/text.py | Storages refuse `..`; `get_valid_filename` keeps `[-\w.]` only |
| Django templates | https://docs.djangoproject.com/en/5.1/topics/templates/ | "The template system isn't safe against untrusted template authors" |
| Django sessions / 5.0 release notes | https://docs.djangoproject.com/en/5.1/topics/http/sessions/, https://docs.djangoproject.com/en/5.1/releases/5.0/ | Pickle sessions turn a key leak into RCE (PickleSerializer removed); `format_html` deprecation |
| DRF filtering guide (`OrderingFilter`) | https://www.django-rest-framework.org/api-guide/filtering/ | Set `ordering_fields` "to prevent unexpected data leakage, such as allowing users to order against a password hash field" |
| DRF `views.py` source and authentication docs | https://raw.githubusercontent.com/encode/django-rest-framework/main/rest_framework/views.py, https://www.django-rest-framework.org/api-guide/authentication/ | `APIView.as_view()` is already `csrf_exempt`; `SessionAuthentication` enforces CSRF itself |
| Flask API (`send_file`, `send_from_directory`, `render_template_string`) | https://flask.palletsprojects.com/en/stable/api/ | "Never pass file paths provided by a user" to `send_file`; `send_from_directory` uses `safe_join`; `render_template_string` source is template code |
| Flask quickstart and security pages | https://flask.palletsprojects.com/en/stable/quickstart/, https://flask.palletsprojects.com/en/stable/web-security/ | A returned string is HTML; `Markup` on user data is an XSS risk |
| Flask uploads pattern, Werkzeug utils | https://flask.palletsprojects.com/en/stable/patterns/fileuploads/, https://werkzeug.palletsprojects.com/en/stable/utils/ | `secure_filename`, `safe_join` |
| Flask-Login docs | https://flask-login.readthedocs.io/en/latest/ | "You MUST validate the value of the next parameter" |
| FastAPI custom responses, CORS, request files | https://fastapi.tiangolo.com/advanced/custom-response/, https://fastapi.tiangolo.com/tutorial/cors/, https://fastapi.tiangolo.com/tutorial/request-files/ | `HTMLResponse` is raw HTML; the CORS page's wildcard/credentials claim; `UploadFile.filename` is client-chosen |
| Starlette `cors.py` source and middleware docs | https://raw.githubusercontent.com/encode/starlette/master/starlette/middleware/cors.py, https://www.starlette.dev/middleware/ | With `"*"` in `allow_origins` and `allow_credentials=True`, Starlette reflects the request's Origin with credentials |
| django-cors-headers README and middleware source | https://raw.githubusercontent.com/adamchainz/django-cors-headers/main/README.rst, https://raw.githubusercontent.com/adamchainz/django-cors-headers/main/src/corsheaders/middleware.py | `CORS_ALLOW_ALL_ORIGINS` with `CORS_ALLOW_CREDENTIALS` reflects the Origin |
| flask-cors `core.py` and API docs | https://raw.githubusercontent.com/corydolphin/flask-cors/main/flask_cors/core.py, https://flask-cors.readthedocs.io/en/latest/api.html | `supports_credentials=True` with the default origins reflects any Origin |
| SQLAlchemy `text()`, tutorial, `exec_driver_sql` | https://docs.sqlalchemy.org/en/20/core/sqlelement.html, https://docs.sqlalchemy.org/en/20/tutorial/dbapi_transactions.html, https://docs.sqlalchemy.org/en/20/core/connections.html | Bound parameters; "a Python literal value … should never be stringified into SQL string directly" |
| Python `subprocess` security, `shlex` | https://docs.python.org/3/library/subprocess.html, https://docs.python.org/3/library/shlex.html | With `shell=True` quoting is the caller's job; `shlex.quote` |
| Python `pickle`, PyYAML, jsonpickle | https://docs.python.org/3/library/pickle.html, https://pyyaml.org/wiki/PyYAMLDocumentation, https://jsonpickle.github.io/ | "Never unpickle data that could have come from an untrusted source"; `yaml.load` "is as powerful as pickle.load" |
| Python `os.path.join`, `pathlib` | https://docs.python.org/3/library/os.path.html, https://docs.python.org/3/library/pathlib.html | An absolute segment drops the base; `is_relative_to` is string-based |
| Python `hmac`, `secrets`, Django `constant_time_compare` | https://docs.python.org/3/library/hmac.html, https://docs.python.org/3/library/secrets.html, https://raw.githubusercontent.com/django/django/main/django/utils/crypto.py | Compare digests with `compare_digest`, "instead of the == operator" |
| GitHub: validating webhook deliveries | https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries | "Never use a plain == operator" for signatures |
| requests / HTTPX / aiohttp redirect behaviour | https://requests.readthedocs.io/en/latest/user/quickstart/, https://www.python-httpx.org/compatibility/, https://docs.aiohttp.org/en/stable/client_reference.html | requests and aiohttp follow redirects by default, so a one-time URL check is bypassed |
| PyJWT usage, changelog, `api_jws.py`; python-jose `jws.py` | https://pyjwt.readthedocs.io/en/stable/usage.html, https://pyjwt.readthedocs.io/en/stable/changelog.html, https://raw.githubusercontent.com/jpadilla/pyjwt/master/jwt/api_jws.py, https://raw.githubusercontent.com/mpdavis/python-jose/master/jose/jws.py | `options={"verify_signature": False}` reads claims without validation; PyJWT 2 requires `algorithms`; python-jose accepts any header `alg` when `algorithms` is omitted |
| Jinja sandbox | https://jinja.palletsprojects.com/en/stable/sandbox/ | The sandbox is mitigation, not a fix, for user templates |

### Security references

| Source | URL | What it contributed |
|---|---|---|
| OWASP SQL Injection Prevention Cheat Sheet | https://cheatsheetseries.owasp.org/cheatsheets/SQL_Injection_Prevention_Cheat_Sheet.html | Parameterise values; allow-list identifiers |
| OWASP OS Command Injection Defense Cheat Sheet | https://cheatsheetseries.owasp.org/cheatsheets/OS_Command_Injection_Defense_Cheat_Sheet.html | Avoid shells; argument injection |
| OWASP SSRF Prevention Cheat Sheet | https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html | Allow-lists; redirects and DNS rebinding bypass one-time checks |
| OWASP Path Traversal Prevention / File Upload cheat sheets | https://cheatsheetseries.owasp.org/cheatsheets/Path_Traversal_Prevention_Cheat_Sheet.html, https://cheatsheetseries.owasp.org/cheatsheets/File_Upload_Cheat_Sheet.html | realpath + commonpath containment; filename rules |
| OWASP Deserialization Cheat Sheet | https://cheatsheetseries.owasp.org/cheatsheets/Deserialization_Cheat_Sheet.html | The Python sinks: pickle, yaml with unsafe loaders, jsonpickle |
| OWASP Unvalidated Redirects Cheat Sheet | https://cheatsheetseries.owasp.org/cheatsheets/Unvalidated_Redirects_and_Forwards_Cheat_Sheet.html | `redirect(request.param)` shape; prefix checks are bypassed |
| OWASP XSS Prevention Cheat Sheet | https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html | Framework escape hatches reintroduce XSS |
| OWASP CSRF Prevention Cheat Sheet | https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html | Exempt only endpoints secured otherwise (webhook signatures) |
| OWASP ReDoS | https://owasp.org/www-community/attacks/Regular_expression_Denial_of_Service_-_ReDoS | Regex injection |
| PortSwigger: SSRF, SSTI, CORS, JWT algorithm confusion | https://portswigger.net/web-security/ssrf, https://portswigger.net/web-security/server-side-template-injection, https://portswigger.net/web-security/cors, https://portswigger.net/web-security/jwt/algorithm-confusion | Attack shapes and bypasses for each sink |
| elttam: plORMbing your Django ORM (2024) | https://www.elttam.com/blog/plormbing-your-django-orm/ | `filter(**request.data)` leaks related fields (`created_by__user__password__startswith`) |
| PayloadsAllTheThings: ORM Leak | https://swisskyrepo.github.io/PayloadsAllTheThings/ORM%20Leak/ | Django ORM-leak payloads |
| Auth0: critical vulnerabilities in JWT libraries | https://auth0.com/blog/critical-vulnerabilities-in-json-web-token-libraries/ | `alg: none`, RS/HS confusion: "attackers control the choice of algorithm" |
| Semgrep: JWT mistakes; semgrep-rules repository | https://semgrep.dev/blog/2020/hardcoded-secrets-unverified-tokens-and-other-common-jwt-mistakes/, https://github.com/semgrep/semgrep-rules | What the registry covers (mostly search-mode, intra-function, Django/Flask only) |
| CodeQL Python query help | https://codeql.github.com/codeql-query-help/python/ | Taint queries for redirect, XSS, regex injection |
| Ruff rules index, Bandit plugin and blacklist index | https://docs.astral.sh/ruff/rules/, https://bandit.readthedocs.io/en/latest/plugins/index.html, https://bandit.readthedocs.io/en/latest/blacklists/blacklist_calls.html | The single-call overlap per candidate |

### Advisories in Python web applications (opened through the GitHub advisory pages)

| Candidate | Advisories |
|---|---|
| SQL injection | GHSA-pwh4-6r3m-j2rf (pyload-ng, f-string `IN ('{…}')`), GHSA-r75f-5x8p-qvmc (litellm, header in query text), GHSA-h7j7-3rx6-xvcg (CKAN), GHSA-8ppf-x4gr-2x7g (calibreweb) |
| ORM lookup injection | GHSA-6hjj-gq77-j4qw / CVE-2023-47117 (Label Studio: user filter keys leaked password hashes), Django CVE-2025-64459, CVE-2021-35042 |
| Command injection | GHSA-g5mq-prx7-c588 (motionEye, `shell=True` with hand-quoted `'{device}'`), GHSA-wwv8-cqpr-vx3m (Modoboa, `shell` set through `**kwargs`), GHSA-6hw7-j4jw-wpff (galaxy_ng), GHSA-79h8-gxhq-q3jg (lollms, FastAPI), GHSA-9fpx-fr6j-2vh6 (GeoNode) |
| Code injection | GHSA-xv5p-fjw5-vrj6 (fugue, `cloudpickle.loads` of request data), GHSA-vqxf-v2gg-x3hc (docling-core, `FullLoader`), GHSA-6wvf-77m9-58rm (litellm, user Jinja template), GHSA-4r7v-whpg-8rx3 (changedetection.io, `from_string`), GHSA-c34r-238x-f7qx (fides) |
| Path traversal | GHSA-9jj8-v89v-xjvw (changedetection.io, `send_from_directory` directory from the route), GHSA-ph9w-r52h-28p7 and GHSA-9whx-c884-c68q (langflow), GHSA-wqmh-qccw-pqwf (mlflow), GHSA-65mp-fq8v-56jr (Flask-Reuploaded) |
| SSRF | GHSA-p59w-9gqw-wj8r (Label Studio), GHSA-h6x2-583h-x99r and GHSA-5x7x-4c3c-qf5w (Open WebUI), GHSA-3c45-4pj5-ch7m (changedetection.io), GHSA-5993-7p27-66g5 (langflow) |
| Open redirect | GHSA-vqv8-j3mj-wjxj (wger, `HttpResponseRedirect(request.GET['next'])`), GHSA-564j-v29w-rqr6 (Khoj, FastAPI `RedirectResponse`), GHSA-8m9j-2f32-2vx4 (MobSF), GHSA-9x43-5qcq-h79q (grappelli `startswith("/")` bypass), GHSA-g3cm-qg2v-2hj5 (pyLoad custom `is_safe_url`) |
| XSS | GHSA-hvq6-2r72-p2x7 (django CMS, interpolate then `mark_safe`), GHSA-g2r2-3j32-j27x (query parameter in an f-string HTML response), GHSA-hhww-mrg2-969h (LiteLLM, `json.dumps` in `<script>`) |
| JWT | CVE-2022-29217 / GHSA-ffqj-6fqr-9h24 (PyJWT key confusion), CVE-2024-33663 (python-jose), GHSA-r9gp-7f88-9r54 (Lemur, `algorithms=[header["alg"]]`), GHSA-mcgx-2gcr-p3hp (jupyterhub-ltiauthenticator, signature not validated) |
| CSRF exemption | CVE-2026-93456 (django-page-cms: `@staff_member_required @csrf_exempt` on admin writes), GHSA-xf64-4pmc-h8qf (wger, login on GET) |
| Timing-unsafe comparison | GHSA-xg4x-w2j3-57h6 (OctoPrint API key), GHSA-j757-pf57-f8r4 (Gradio), GHSA-f3q4-ggfp-jv34 (Adyen webhook HMAC), GHSA-jjjr-3jcw-f8v6 (Wagtail page password), GHSA-m38j-pmg3-v5x5 (django-basic-auth-ip-whitelist) |
| CORS | GHSA-9jfm-9rc6-2hfq and GHSA-fp27-88fp-2phg (Glances, FastAPI), GHSA-6x6h-qqr7-855w (LightRAG), GHSA-g9rg-8vq5-mpwm (mcp-memory-service), GHSA-4xqm-4p72-87h6 (Sentry, suffix match) |
| Regex injection | GHSA-2f54-p244-32q6 (Open WebUI), GHSA-qrpw-gjvh-x5gm (Nautobot), GHSA-8w9j-hc3g-3g7f (PraisonAI) |
| Archive extraction | GHSA-fhff-qmm8-h2fp (mlflow, tarfile), GHSA-mvwx-582f-56r7 (pyload-ng); GHSA-25g8-2mcf-fcx9 (changedetection.io, zipfile) contradicts the stdlib docs and a local test |

## 2. Candidates

Overlap probe (Ruff 0.17.6 `--select S`, Bandit 1.9.4, a scratch file with a vulnerable and a safe line per shape):
S608/B608 flag any SQL built by formatting, from any source, and also the safe `IN ({placeholders})` shape; S602/B602
flag a `shlex.quote`d value under `shell=True` and miss `shell` passed through `**kwargs`; S301/B301 flag every pickle
call and miss `cloudpickle`; S202/B202 report `zipfile` extraction as tarfile. Neither tool has a check for ORM
lookups, SSRF through requests/httpx/aiohttp, path traversal, template injection, open redirects, JWT, CORS, CSRF
exemption or timing-unsafe comparison.

"Probe" is what a text search of the 27 corpus repositories and a run of the library's sink probe (every call to a
sink-named function whose argument the library's taint reaches) found before writing the rule.

| Id | What it catches | Why it is a bug | Sources | Files | Linter overlap | Engine detection | Probe | Verdict |
|---|---|---|---|---|---|---|---|---|
| `sql-injection` | Request data spliced (f-string, `%`, `+`, `.format`, `.join`) into the SQL text of `raw()`, `extra()`, `RawSQL`, `cursor.execute`, SQLAlchemy `text()`, `exec_driver_sql`, `session.execute(str)` | The client writes SQL: data theft or loss | Django raw SQL docs, SQLAlchemy docs, OWASP SQLi; pyload-ng, litellm, CKAN | cross-file: views hand values to query helpers | S608/B608 flag every formatted SQL string (and the safe placeholder-list shape); S610/S611 any `extra`/`RawSQL` | `sec_built` reaching the SQL argument; params position ignored | 375 raw-SQL call sites; probe: parameterised uses only so far | build |
| `orm-lookup-injection` | A request-controlled mapping expanded as `**` into `filter/exclude/get/annotate/alias/values/Q`, a request key used as a lookup name, or a request string passed to `order_by()`/`values()` without an allow-list | The client picks the lookup: filters on `user__password__startswith` leak hidden fields; on unpatched Django, SQL injection | elttam plORMbing, Django security releases, DRF OrderingFilter docs; Label Studio CVE-2023-47117 | cross-file | none (Ruff, Bandit, Semgrep) | `sec_tainted` on a `**` argument or on order_by's arguments; `in`-checked and dict-mapped values skipped | probe: `filter(**…)` from request data to be counted | build |
| `command-injection` | Request data in a shell command (`shell=True` string, `os.system`, `os.popen`, `asyncio.create_subprocess_shell`, `shell` via `**kwargs`) or as the program of an argument list | Remote command execution | subprocess docs, OWASP command injection; motionEye, Modoboa, galaxy_ng, lollms, GeoNode | cross-file (wrappers) | S602/S605/B602/B605 flag every formatted command (and `shlex.quote`d ones), miss `**kwargs` | `sec_tainted` into the command argument; `shlex.quote` stops it | no request-fed shell call found by text search; strong advisory evidence | build |
| `code-injection` | Request data reaching `eval`/`exec`, `pickle`/`cloudpickle`/`dill`/`marshal`/`jsonpickle` loads, `yaml.load` with an unsafe loader, a template's source (`render_template_string`, `jinja2.Template`, `Environment.from_string`, Django `Template`/`Engine.from_string`), or `importlib.import_module` | Remote code execution | pickle and PyYAML docs, OWASP deserialization, PortSwigger SSTI, Django templates; fugue, docling-core, litellm, changedetection.io | cross-file | S301/B301 and S506/B506 flag every call (miss cloudpickle); nothing for SSTI | `sec_tainted` into the payload or template-source argument | probe: `json.loads` only so far | build |
| `path-traversal` | Request data, an upload's filename included, in a file path: `open`, `pathlib` read/write, `os.remove`, `shutil`, Flask `send_file`, the directory of `send_from_directory`, Starlette/Django `FileResponse(path)` | Arbitrary file read, write or delete | Flask `send_file` docs, OWASP path traversal, `os.path.join` docs; changedetection.io, langflow, mlflow | cross-file | none | `sec_tainted` into the path; `basename`, `secure_filename`, `get_valid_filename`, `safe_join`, a resolve-and-contain check stop it | probe: one `open` of an upload's content (a file object, not a path) | build |
| `ssrf` | Request data as the URL (scheme and host) of requests, httpx, aiohttp or urllib calls | The server fetches internal services and cloud metadata for the client | OWASP SSRF, PortSwigger SSRF, requests/aiohttp redirect docs; Label Studio, Open WebUI, changedetection.io, langflow | cross-file (HTTP client wrappers) | S310/B310 audit only `urlopen`'s scheme | `sec_tainted` into the URL, but not when it is only the path or query of a fixed base | probe: linkding, healthchecks, pretix send request-derived URLs | build |
| `open-redirect` | Request data passed to `redirect()`, `HttpResponseRedirect`, `RedirectResponse` or a `Location` header without `url_has_allowed_host_and_scheme` | Phishing through the site's own domain; token leaks in OAuth flows | Django `url_has_allowed_host_and_scheme`, Flask-Login docs, OWASP redirects; wger, Khoj, MobSF | cross-file (`get_success_url`, `next` helpers) | none | `sec_tainted` into the redirect target, unless the value or a function returning it is checked (`sec_checked_expr`) | probe: 17 request-fed redirects in pretix, label-studio, healthchecks, linkding, every one checked | build |
| `xss-unescaped-html` | Request data reaching `mark_safe`, `SafeString`, `Markup(...)`, `format_html`'s format string, `HttpResponse` with an HTML type, `HTMLResponse`, or a Flask route's returned string | Stored or reflected XSS | Django `format_html`/`mark_safe`, MarkupSafe docs, Flask quickstart, OWASP XSS; django CMS advisory | cross-file | S308/B308/B703 flag every `mark_safe`, S704/B704 every non-literal `Markup` | `sec_tainted` into the HTML argument; `escape`, `conditional_escape`, `format_html` args stop it | to be counted | build |
| `jwt-unverified` | `jwt.decode` with `verify_signature: False` (or PyJWT 1's `verify=False`) whose claims decide who the caller is, `algorithms` taken from the token's own header, python-jose `decode` without `algorithms` | Authentication bypass: forged tokens accepted | PyJWT docs and changelog, python-jose source, Auth0, PortSwigger; Lemur, PyJWT and python-jose CVEs | single-file (data flow inside the auth function) | none | the decode call's options and `algorithms` argument; flow of `get_unverified_header` into `algorithms` | label-studio, superset decode unverified to sniff a token's format (not auth) | build, expect log |
| `cors-credentials-any-origin` | Starlette/FastAPI `CORSMiddleware` with `"*"` in `allow_origins` (literal or a settings default) or `allow_origin_regex` matching anything, with `allow_credentials=True`; django-cors-headers allow-all with credentials; flask-cors `supports_credentials=True` with any origin | Any site reads authenticated responses: the middleware reflects the attacker's Origin | Starlette `cors.py`, django-cors-headers source, flask-cors source, PortSwigger CORS; Glances, LightRAG, mcp-memory-service | cross-file (origins from a settings module) | none (Semgrep flags `"*"` without regard to credentials) | the middleware call's arguments, following names to their settings value or environment default | open-webui: `os.getenv('CORS_ALLOW_ORIGIN', '*')` with credentials | build |
| `csrf-exempt-session-write` | `@csrf_exempt` on a Django function or class view that authenticates by session (`login_required`, `request.user`) and writes, with no signature check | Cross-site requests act as the logged-in user | Django CSRF docs, OWASP CSRF, DRF views source; django-page-cms CVE-2026-93456 | cross-file (writes through helpers) | none (Semgrep flags every `csrf_exempt` at low confidence) | decorator, session evidence, write reach through the call graph, absence of `compare_digest`/`construct_event` | 68 `@csrf_exempt` in the corpus, most on token or webhook endpoints | build, expect log |
| `timing-unsafe-compare` | `==`/`!=` between request data and a secret: an HMAC or hash digest, a name like `*secret*`, `*token*`, `*signature*`, `*api_key*`, a settings secret | The secret leaks one byte at a time through response timing | Python `hmac` docs, GitHub webhook docs; OctoPrint, Gradio, Adyen, Wagtail | cross-file | none (Ruff, Bandit, Semgrep, CodeQL) | `sec_tainted` on one side, a secret-like value on the other | 83 `compare_digest`/`constant_time_compare` uses: teams know the idiom | build |
| `format-html-interpolated` | `format_html(f"…")` or with a `%`/`.format()` format string | Escaping is skipped for the interpolated values; no-arg calls are deprecated (5.0) | Django utils docs, 5.0 release notes | single-file | Semgrep `formathtml-fstring-parameter` | syntax | one corpus hit, a template tag interpolating a tag name on purpose | drop: Django itself warns and then refuses the no-argument form; one deliberate corpus occurrence; a request-data case is caught by xss-unescaped-html |
| `regex-injection` | Request data as a regular expression pattern without `re.escape` | ReDoS stalls a worker | OWASP ReDoS, Python `re` docs; Open WebUI, Nautobot | cross-file | none | `sec_tainted` into the pattern argument | probe: request data only in the subject string (pretix cookies, healthchecks) | drop: availability only, rare, and every probe hit was the safe subject side |
| `zip-slip` | Archive extraction of uploads without a filter | Arbitrary file write | tarfile docs, PEP 706; mlflow, pyload-ng | single-file | S202/B202 (tarfile) | — | — | drop: Bandit and Ruff cover tarfile; `zipfile` already strips `..` (docs, local test) |
| `mass-assignment` | `Model.objects.create(**request.data)` | Clients set fields they should not | Semgrep `mass-assignment` | — | — | — | — | drop: belongs to the drf pack (`input.mass-assignment`) |
| `host-header` | Reading the Host header instead of `get_host()` | Poisoned links | Django docs | — | — | — | — | drop: the django pack ships `django-practice-host-header-without-get-host` |
| `debug-secret-settings` | `DEBUG = True`, hard-coded `SECRET_KEY` | Information leak, forged sessions | Django deployment checklist | single-file | Bandit B105, Django `check --deploy` | — | — | drop: Django's deploy checks and Bandit enforce it |
| `requests-verify-false` | `verify=False` on HTTP calls | MITM | requests docs | single-file | Bandit B501, Ruff S501 | — | — | drop: linter covers it |
| `xxe` | lxml/ElementTree parsing request bodies | XXE, entity expansion | defusedxml docs | single-file | Bandit B313-B320, B405-B410 | — | — | drop: the python pack ports the Bandit XML checks |
| `insecure-cookie` | `set_cookie` of a session token without `secure`/`httponly` | Token theft | OWASP session management | single-file | none | — | — | drop: Django's `SESSION_COOKIE_*` settings and `check --deploy` cover the session cookie; custom cookies vary too much to steer safely |
| `log-injection` | Request data in log messages | Forged log lines | OWASP logging | — | — | — | — | drop: low impact, high volume |

## 3. Build list, ranked by value

All twelve are `security` risks; they are ranked by the worst realistic outcome, then by how often teams hit it.

1. `python-security-sql-injection`: data theft and loss through the database (critical).
2. `python-security-command-injection`: remote command execution (critical).
3. `python-security-code-injection`: remote code execution through deserializers, templates and `eval` (critical).
4. `python-security-path-traversal`: arbitrary file read, write or delete (critical).
5. `python-security-jwt-unverified`: authentication bypass (critical).
6. `python-security-orm-lookup-injection`: hidden-field leaks through the ORM, SQL injection on unpatched Django (high).
7. `python-security-ssrf`: internal network and cloud-metadata access (high).
8. `python-security-cors-credentials-any-origin`: any site reads authenticated responses (high).
9. `python-security-xss-unescaped-html`: XSS through the frameworks' escape hatches (high).
10. `python-security-csrf-exempt-session-write`: cross-site writes as the logged-in user (high).
11. `python-security-open-redirect`: phishing and OAuth token leaks through the site's domain (medium).
12. `python-security-timing-unsafe-compare`: secrets leaked through timing (medium).
