# Notes: the Django pack's stack additions (migrations, settings, ORM)

Mining notes for the rules added to the 27 of [NOTES-django.md](NOTES-django.md), for a Django + PostgreSQL team. The
probes below ran on the 10 Django repositories of `~/rerail-bench/python-arch/corpus` (MANIFEST.tsv: linkding,
healthchecks, paperless-ngx, label-studio, plane, InvenTree, pretix, addons-server, DefectDojo, saleor), with
throwaway CPython `ast` scripts and text searches (3,726 migration files; DefectDojo keeps its migrations in
`dojo/db_migrations/`). Counts are from those scripts, not from the rules; each rule's own corpus numbers are in
README.md's "Rule details".

## 1. Sources read

| Source | URL | What it contributed |
|---|---|---|
| Django docs, How to create database migrations ("Migrations that add unique fields") | https://docs.djangoproject.com/en/5.2/howto/writing-migrations/ | An AddField default is computed once for every existing row; the three-step add/backfill/alter recipe |
| django.wiki, Add unique UUIDs to existing rows | https://django.wiki/guides/django-data-migration-unique-uuid | A second write-up of the one-value-for-all-rows trap and the three-migration fix |
| CWE-1188, Initialization of a Resource with an Insecure Default | https://cwe.mitre.org/data/definitions/1188.html | The fail-open default as a weakness class |
| Django ticket #37031 | https://code.djangoproject.com/ticket/37031 | Maintainers on the same recipe; `db_default` makes the database compute a value per row |
| Django docs, Migration Operations (RunPython) | https://docs.djangoproject.com/en/5.2/ref/migration-operations/ | "avoid combining schema changes and RunPython operations in the same migration or you may hit … pending trigger events"; RunPython gets historical models |
| Django docs, Migrations ("Historical models") | https://docs.djangoproject.com/en/5.2/topics/migrations/ | "If you import models directly rather than using the historical models, your migrations may work initially but will fail in the future" |
| Zulip, Schema migrations | https://zulip.readthedocs.io/en/stable/subsystems/schema-migrations.html | Why the trigger error happens (Django's FKs are DEFERRABLE INITIALLY DEFERRED), and the fix (split, or non-atomic) |
| Stack Overflow 12838111 | https://stackoverflow.com/questions/12838111/django-db-migrations-cannot-alter-table-because-it-has-pending-trigger-events | Teams hitting the trigger error on real data that tests did not have |
| Django docs, QuerySet API (`bulk_create`) | https://docs.djangoproject.com/en/5.2/ref/models/querysets/ | "Enabling the ignore_conflicts parameter disables setting the primary key on each model instance" |
| Django Forum, rows actually inserted with ignore_conflicts | https://forum.djangoproject.com/t/get-count-of-rows-actually-inserted-using-bulk-create-with-ignore-conflicts/38435 | The returned list is the input list, not the inserted rows |
| Django ticket #34698 | https://code.djangoproject.com/ticket/34698 | IDs are returned for update_conflicts, still not for ignore_conflicts |
| Django docs, Settings (DEBUG, ALLOWED_HOSTS, SECRET_KEY) | https://docs.djangoproject.com/en/5.2/ref/settings/ | "Never deploy a site into production with DEBUG turned on"; `'*'` makes the project responsible for Host validation |
| Django docs, Deployment checklist | https://docs.djangoproject.com/en/5.2/howto/deployment/checklist/ | Load SECRET_KEY from the environment; `check --deploy` reports only the values of the environment it runs in |
| PostgreSQL docs, ALTER TABLE (notes) | https://www.postgresql.org/docs/current/sql-altertable.html | A column type change rewrites the table and its indexes unless the old type is binary coercible to the new |
| Squawk, changing-column-type | https://squawkhq.com/docs/changing-column-type | int → bigint rewrites; varchar → text and widening varchar do not |
| Django 3.2 release notes (DEFAULT_AUTO_FIELD) | https://docs.djangoproject.com/en/5.2/releases/3.2/ | Why AutoField → BigAutoField AlterFields appear in many apps |
| django-migration-linter source (main) | https://github.com/3YOURMIND/django-migration-linter | Exactly what it checks: `sql_analyser/base.py` NOT_NULL (with DROP DEFAULT and db_default), DROP_COLUMN, RENAME_COLUMN, ALTER_COLUMN (any type change), ADD_UNIQUE (ADD CONSTRAINT UNIQUE and CREATE UNIQUE INDEX); `postgresql.py` CREATE_INDEX, DROP_INDEX; `migration_linter.py` RUNPYTHON_REVERSIBLE, RUNPYTHON_MODEL_IMPORT (a regex on the RunPython function's own source), RUNSQL_REVERSIBLE |
| pylint-django | https://github.com/pylint-dev/pylint-django | W5197 missing-backwards-migration-callable |
| Ruff rules (PLW1508 invalid-envvar-default, DJ001-DJ013, S105) | https://docs.astral.sh/ruff/rules/ | What Ruff already says about env defaults and Django models; the `python` pack ports S105/B105 but no DJ or PL rules |
| Django docs, Middleware ordering | https://docs.djangoproject.com/en/5.2/ref/middleware/#middleware-ordering | The documented orderings (dropped candidate) |
| Django docs, `prefetch_related` | https://docs.djangoproject.com/en/5.2/ref/models/querysets/#prefetch-related | A chained filter on a prefetched relation runs a new query (dropped candidate) |

## 2. Candidates

"Overlap" names the tool that already reports the shape. Detection is in the engine's terms (`rules/lib/django.rerail`
gained a migrations section for these rules).

| id | Catches | Why it is a bug | Sources | Scope | Linter overlap | Detection | Verdict |
|---|---|---|---|---|---|---|---|
| settings-env-fails-open | A settings module reads DEBUG, ALLOWED_HOSTS or SECRET_KEY from the environment with a fallback that is insecure when the variable is unset: DEBUG true, `"*"` in ALLOWED_HOSTS, a literal SECRET_KEY | A deploy that forgets the variable runs with debug pages (settings and source leaked), any Host header (reset-link poisoning), or a public signing key (forged sessions and reset tokens) | settings docs; deployment checklist; CWE-1188 | cross-file: env helpers (`envbool`, `get_bool_from_env`, django-environ, python-decouple) resolved into the module that defines them | `check --deploy` W018/W009/W020 see only the evaluated value where it runs and never `'*'`; Ruff PLW1508 only flags a non-str `os.getenv` default; S105 only a literal assignment | settings documents (by name, or holding Django settings), the call that reads the env var, its default argument, helper defs that read os.environ | build |
| addfield-default-runs-once | AddField on an existing table whose default is a per-row generator (uuid4, secrets, random tokens, a project function that calls one) or that is unique | Django evaluates the default once and writes it into every existing row: a unique field fails the migration; a non-unique secret/token is shared by all existing rows (pretix `web_secret`, `session_token`) | writing-migrations docs; ticket #37031; django.wiki guide | cross-file: the default's function in the models module; the CreateModel in an earlier migration | none (default-called-once covers `default=uuid4()` in models, a different bug) | dj_mig_db_op AddField, dj_mig_field, its default, dj_mig_creates | build |
| bulk-ignore-conflicts-used | Objects passed to or returned by `bulk_create(..., ignore_conflicts=True)` used afterwards as saved rows: pk read, used as FK values, dispatched, emailed, counted | Conflicting rows were not inserted; with a serial pk the objects have pk None, with a UUID default pk they point at rows that do not exist; the stored row may differ (plane emails invitation tokens that were never stored) | QuerySet docs; Django forum; ticket #34698; plane and label-studio code comments | single-file, model pk type cross-file | none | the call, its first argument and result names, later reads in the same def | build |
| alterfield-rewrites-table | AlterField whose column type change rewrites the PostgreSQL table (int → bigint incl. AutoField → BigAutoField, a narrower varchar, text → varchar, numeric precision, cross-type) on a table an earlier migration created | ACCESS EXCLUSIVE lock for the whole rewrite: reads and writes stop on a large table | PostgreSQL ALTER TABLE notes; Squawk; Django 3.2 notes | cross-file: the field's previous definition from the app's earlier migrations (dj_mig_before) | django-migration-linter ALTER_COLUMN flags every type change ("could be backward compatible, you may ignore"); Squawk on SQL. Value added: the previous type from history, so widening varchar and varchar → text stay silent | dj_mig_field across dj_mig_before | build (log) |
| runpython-with-schema-change | An atomic migration that writes rows with RunPython and changes the schema of the same model's table | PostgreSQL refuses ALTER TABLE with pending deferred FK trigger events (passes on an empty test database, fails on production data); a schema change before the backfill holds its ACCESS EXCLUSIVE lock for the whole backfill | RunPython docs; Zulip; SO 12838111 | migration file + RunPython function (may live in another module) | none | dj_mig_db_op order within d, dj_runpython, get_model names, !dj_mig_nonatomic | build |
| runpython-live-model | RunPython code that reaches the app's live model classes through a function in another module (a helper in `utils`, `tasks.functions`) | The live model has the current fields, not the migration's: the migration breaks once the model changes, or writes columns that do not exist yet | topics/migrations "Historical models"; RunPython docs | cross-file: call graph from the RunPython function | django-migration-linter RUNPYTHON_MODEL_IMPORT checks `X.objects` in the function's own source only; the direct shape is left to it | dj_runpython + python/calls reach + live model use | build |
| runpython-no-reverse | RunPython without reverse_code | Migration cannot be unapplied | RunPython docs | single-file | django-migration-linter RUNPYTHON_REVERSIBLE; pylint-django W5197 | — | drop: linters cover it (NOTES-django.md agrees) |
| migration-imports-model (direct) | RunPython using an imported model class in its own body | as above | as above | single-file | RUNPYTHON_MODEL_IMPORT | — | drop: linter covers it; the cross-file form is runpython-live-model |
| addfield-not-null-no-db-default | NOT NULL column added without `db_default` | Old code inserting during deploy violates NOT NULL once Django drops the default | — | single-file | django-migration-linter NOT_NULL (detects the DROP DEFAULT, knows db_default since 5.x) | — | drop: linter covers it |
| unique-not-concurrent | unique=True, AddConstraint(UniqueConstraint), AlterUniqueTogether on an existing table | Unique index built under lock | — | single-file | django-migration-linter ADD_UNIQUE (`ALTER TABLE … ADD CONSTRAINT … UNIQUE` and `CREATE UNIQUE INDEX`, tables created in the migration skipped). NOTES-django.md said the linter misses ADD CONSTRAINT UNIQUE; its current source has had ADD_UNIQUE since its changelog's "Detect adding a UNIQUE constraint" entry | — | drop: linter covers it |
| index-not-concurrent | non-concurrent index on an existing table | as NOTES-django.md | — | — | CREATE_INDEX | — | drop: no new evidence |
| field-removed-still-read | RemoveField/RenameField while live code reads the old name | Queries fail | — | cross-file | DROP_COLUMN, RENAME_COLUMN, DROP_TABLE, RENAME_TABLE flag every such migration; a consistent commit that still reads a removed field fails system checks or the first test of that path | text probe: removed/renamed names (over 7 characters) still appearing in live code: 236 lines in 6 repos, the ones read all other models' fields, serializer keys or annotations of the same name | drop: nothing a type-blind matcher finds is real; deploy order is the linter's |
| security-middleware-order | SecurityMiddleware missing, CORS/GZip/Locale out of documented order | Headers not set, wrong body handling | middleware ordering docs; django-cors-headers README | single-file | `check --deploy` W001/W002/W003 for missing ones | MIDDLEWARE list extraction: 11 lists; no hard violation, soft ones only (linkding and pretix put LocaleMiddleware after CommonMiddleware, pretix puts SecurityMiddleware late) | drop: no outage shape occurs; soft orderings would be noise |
| cookie-hsts-settings | SESSION_COOKIE_SECURE, CSRF_COOKIE_SECURE, HSTS off | Cookies over http | deployment checklist | single-file | `check --deploy` W004/W008/W012/W016 | — | drop: Django's own check; the env-default form is folded into settings-env-fails-open only for DEBUG/ALLOWED_HOSTS/SECRET_KEY, where an unset variable is a breach |
| csrf-exempt-state-changing | `@csrf_exempt` on a POST view | CSRF on session-authenticated views | Django CSRF docs | cross-file (urls.py wrappers) | none | 22 sites read: webhooks (GitHub, Jira, Telegram, Stripe, PayPal), signed-token unsubscribe links, GraphQL/API with token auth, a CSP report endpoint; none a session-authenticated form view | drop: 0 real of 22 |
| debug-env-string-truthy | `DEBUG = os.environ.get("DEBUG", False)` (a string, so "False" is truthy) | Debug on in prod | many blog posts | single-file | Ruff PLW1508 (`os.getenv` with a non-str default) | 16 env reads with a bool default: 11 linkding (compared with `in (...)`, safe), plane's tokens (presence tests, safe), 1 real (plane `SCOUT_MONITOR`, not a Django setting) | drop: 1 real; the fail-open half is settings-env-fails-open |
| prefetch-lost-by-filter | `obj.rel.filter()` on a prefetched relation | Prefetch wasted, N+1 | prefetch_related docs | cross-file in practice (DRF view queryset → serializer method) | none | loops over a prefetched queryset that filter the relation: 0 in 10 repos | drop: 0 in the single-function shape; the serializer form belongs to the drf pack |
| sliced-update | `qs[:n].update()` / `.delete()` | TypeError at once | QuerySet docs | single-file | — | 0 | drop: crashes on first run; 0 in corpus |
| distinct-order-by-related | `.order_by("rel__x").distinct()` | Duplicate rows | QuerySet docs (distinct note) | single-file | — | 1 match, a correct `distinct("url__host")` (DISTINCT ON) | drop: 0 real |
| iterator-after-prefetch | `.prefetch_related().iterator()` without chunk_size | Ignored prefetch; ValueError since Django 5.0 | Django 4.1 deprecation | single-file | — | 0 | drop: 0, and it raises on Django ≥ 5.0 |
| format-html-fstring | `format_html(f"...")` | XSS, deprecated without args | Django 5.0 notes | single-file | — | 1 (pretix, a tag name from a template tag) | drop: 1 occurrence |
| runpython-custom-manager | historical model used with a custom manager method | AttributeError | topics/migrations | cross-file | — | — | drop: fails on its first run (tests run migrations) |

## 3. Build list, ranked by value

1. `django-practice-settings-env-fails-open` (security): a missing variable on one deploy exposes debug pages, any
   Host or a public signing key. Corpus shapes: healthchecks `SECRET_KEY = envsecret("SECRET_KEY", "---")` and
   `DEBUG = envbool("DEBUG", "True")`, saleor `DEBUG = get_bool_from_env("DEBUG", True)`, label-studio
   `DEBUG = get_bool_env('DEBUG', True)` and `ALLOWED_HOSTS = get_env_list('ALLOWED_HOSTS', default=['*'])`, plane
   `ALLOWED_HOSTS = os.environ.get("ALLOWED_HOSTS", "*").split(",")`.
2. `django-practice-addfield-default-runs-once` (data safety, security): every existing row gets one generated value.
   Corpus: 18 AddFields with a callable default on an existing table, among them pretix `0122_orderposition_web_secret`
   and `0147_user_session_token` (one secret for every existing order position and user).
3. `django-practice-bulk-ignore-conflicts-used` (silent wrong results): 89 `ignore_conflicts=True` calls outside tests,
   27 whose objects or result are read afterwards (plane 14, DefectDojo 7, saleor 4, label-studio 1, addons-server 1),
   e.g. plane `app/views/project/invite.py:114` emails each returned invitation's token.
4. `django-practice-alterfield-rewrites-table` (outage, Log): 84 AlterFields to BigAutoField (pretix 81) and 4 to
   BigIntegerField, beside other type changes the history must classify.
5. `django-practice-runpython-with-schema-change` (outage at deploy): 112 atomic migrations whose RunPython writes rows
   of a model (by `get_model`) that a later operation of the same migration alters, in 7 repos.
6. `django-practice-runpython-live-model` (migration breaks later): RunPython functions calling project helpers in
   other modules: label-studio `tasks.functions.calculate_stats_all_orgs`, `fill_annotations_project`,
   `fill_predictions_project`, plane `plane.utils.filters.filter_migrations`, DefectDojo `dojo.vulnerability.backfill`,
   linkding `bookmarks.utils.normalize_url` (no model: must stay silent).

Six, not eight: the other focus items either are django-migration-linter's checks already (NOT NULL, unique
constraints, indexes, RunPython reversibility, direct model imports) or have no real occurrence in the corpus
(the ORM footguns, CSRF exemptions, middleware order).
