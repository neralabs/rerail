# django-practice-get-handler-writes

Invariant: a view handler that answers only safe methods (GET/HEAD/OPTIONS) does not write model rows, itself or
through what it calls.

## Sources

- Django docs, Cross Site Request Forgery protection (https://docs.djangoproject.com/en/stable/ref/csrf/, source
  docs/ref/csrf.txt): "The first defense against CSRF attacks is to ensure that GET requests (and other 'safe'
  methods, as defined by :rfc:`9110#section-9.2.1`) are side effect free." and "It deliberately ignores GET and
  QUERY requests (and other requests that are defined as 'safe' by :rfc:`9110#section-9.2.1`). These requests ought
  never to have any potentially dangerous side effects, and so a CSRF attack with a GET request ought to be
  harmless."
- RFC 9110 §9.2.1 Safe Methods (https://www.rfc-editor.org/rfc/rfc9110#section-9.2.1): "Request methods are
  considered "safe" if their defined semantics are essentially read-only; i.e., the client does not request, and
  does not expect, any state change on the origin server as a result of applying a safe method to a target
  resource." The same section allows incidental effects: "most servers append request information to access log
  files at the completion of every response, regardless of the method, and that is considered safe".

## Mode

steer: 8/8 read matches real (the bar's ≥98%). Sample is small (14 of 32 repos finished); if the remaining big repos
bring many lazy-init/history writes the team does not want steered, drop to log.

## Linter coverage

None. Ruff (DJ, S), flake8-django, pylint-django, Bandit: no rule ties an HTTP method to database writes (checked
the rule lists: https://docs.astral.sh/ruff/rules/, https://github.com/rocioar/flake8-django#list-of-warnings,
https://github.com/pylint-dev/pylint-django#features, https://bandit.readthedocs.io/en/latest/plugins/index.html).
Django system checks do not inspect view bodies.

## What it matches

Handlers: for each dj_view_class K, the workspace method K resolves get/head/list/retrieve to (own or inherited) (Django View/generic views, DRF views and
viewsets, through the run's classes), and functions decorated `@require_GET`, `@require_safe`,
`@require_http_methods([...])` / `@api_view([...])` listing only GET/HEAD/OPTIONS (any case), or `@api_view()`.
Writes: QuerySet create/update/delete/get_or_create/update_or_create/bulk_create/bulk_update (and a-twins), and
save()/delete() (and a-twins) on a known model instance, in the handler or in any function it reaches through the
call graph (python/calls across files, plus `obj.m()` on a known model instance resolved to the model's method).
`self.m()` in K's methods goes to K's m (mro_method). One finding per handler naming one write and the function
holding it: at the `def` for K's own method or a function view; at `class K` for an inherited handler, only for
writes K's overrides add.

Misses: writes through related managers (`obj.tags.add`), custom managers, `form.save()`, raw SQL; handlers chosen
by URLconf or `http_method_names` rather than by name/decorator; calls with unknown targets.

## Corpus

Final version finished on 11 repos (umap, mathesar, linkding, python-discord, Kiwi, paperless-ngx, pontoon,
djangoproject.com, healthchecks, taiga-back, arches): 7 distinct findings (8 rows; taiga NotifyPolicyViewSet is
reported for both inherited list and retrieve on one line) in 3 repos; read all 7, real 7, precision 100%. An earlier
version (no class-keyed self dispatch) also finished inventree, cvat, plane, label-studio, oscar: 1 more match
(inventree, real), 0 in the others. Not run before the deadline (timeouts): readthedocs, ralph, awx, pretix,
nautobot, Weblate, netbox, LibrePhotos, addons-server, Misago, zulip, authentik, DefectDojo, saleor, openedx,
PostHog.

Examples (real):
- python-discord__site:pydis_site/apps/home/views.py:184 (`get` refreshes cached repo metadata with bulk_create)
- taigaio__taiga-back:taiga/projects/settings/api.py:48 (`list` lazily get_or_create's per-user settings)
- archesproject__arches:arches/app/views/api/search.py:26 (`get` saves a SearchExportHistory row)
- inventree__InvenTree:src/backend/InvenTree/users/api.py:427 (`get` deletes and creates an API token)

FP class found and fixed: late-bound `self.m()` resolved to every subclass override, so taiga's generic
ListAPIView/RetrieveAPIView.get were reported for writes in some subclass's get_queryset (6 FPs). Now a handler is
(view class K, the method K resolves get/list/... to), and `self.m()` in K's methods goes to K's m; a class that
inherits the handler is reported at `class K` only for writes its own overrides add (cases: inherited.py).

Expected noise classes (RFC 9110 calls incidental logging safe): lazy-init get_or_create, export/visit history,
cache refresh. All are the pattern; none seen that is not a write on the GET path.

Runtime: dominated by the python/calls typing program (see django-practice-sync-orm-reached-from-async notes).
Misses also come from a library gap: two-dot relative imports (`from .. import BaseAPIView`) do not resolve, so
those views are not dj_view_class (makeplane/plane's CycleUserPropertiesEndpoint.get with get_or_create).

## Helpers worth a library

`ghw_instance_class`, `ghw_qs_model`, `ghw_fetch_call`, `ghw_model_call` (same as the sibling cross-file rule
django-practice-sync-orm-reached-from-async's `sora_*`): python/django.rr's `dj_instance_class(r, h, q)`,
`dj_qs_model(e, q)`, `dj_model_call(c, g)`. `ghw_handler` (a view function/method answering only safe methods)
would serve any "GET handler" rule: python/django.rr `dj_safe_handler(f)`.
