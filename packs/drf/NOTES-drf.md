# Django REST framework pack: mining notes

What the `drf-practice-*` rules are built from: the sources read, every candidate considered with its verdict, and the
build list. The django pack (`packs/django/`) already covers generic Django ORM, transaction and view rules; nothing here
repeats one of them. Each candidate was checked against the `python` pack's Ruff 0.16.9 and Bandit 1.9.4 ports
(`packs/python/pack.json`) and the stack's own linters (below).

## 1. Sources read

| Source | URL | What it contributed |
|---|---|---|
| DRF API guide: Serializers | https://www.django-rest-framework.org/api-guide/serializers/ | "strongly recommended that you explicitly set all fields … less likely to result in unintentionally exposing data"; `validate_<field>` "should return the validated value"; `read_only_fields` and `extra_kwargs` do not apply to a field declared on the class (or a parent); writable nested serializers need explicit `create()`/`update()` |
| DRF API guide: Permissions | https://www.django-rest-framework.org/api-guide/permissions/ | `DEFAULT_PERMISSION_CLASSES` defaults to `AllowAny`; object permissions run only when `get_object()` or `check_object_permissions()` runs; "generic views will only check the object-level permissions for views that retrieve a single model instance"; function views must check object permissions themselves |
| DRF API guide: Generic views | https://www.django-rest-framework.org/api-guide/generic-views/ | `queryset` "will get evaluated once, and those results will be cached for all subsequent requests" when read directly; "Avoiding N+1 Queries": optimise the view's queryset with `select_related()`/`prefetch_related()` for the relations the serializer reads; `perform_create()` with `serializer.save(user=self.request.user)` |
| DRF API guide: Pagination | https://www.django-rest-framework.org/api-guide/pagination/ | `DEFAULT_PAGINATION_CLASS` and `PAGE_SIZE` are None by default (no pagination); `page_size_query_param` lets the client set the page size, `max_page_size` caps it; `LimitOffsetPagination.max_limit` defaults to None |
| DRF API guide: Filtering ("Filtering against the current user") | https://www.django-rest-framework.org/api-guide/filtering/ | scoping `get_queryset()` by `request.user`, the companion of object permissions for list views |
| DRF API guide: Settings | https://www.django-rest-framework.org/api-guide/settings/ | the defaults of `DEFAULT_PERMISSION_CLASSES`, `DEFAULT_PAGINATION_CLASS`, `PAGE_SIZE` |
| DRF API guide: Fields | https://www.django-rest-framework.org/api-guide/fields/ | `write_only`, `read_only`, `HiddenField`, `CurrentUserDefault`, `SerializerMethodField` semantics |
| DRF 3.3 announcement | https://www.django-rest-framework.org/community/3.3-announcement/ | `fields`/`exclude` became mandatory on ModelSerializer because implicit "all fields" leaked data |
| DRF source, `serializers.py` | https://github.com/encode/django-rest-framework/blob/master/rest_framework/serializers.py | `run_validation` asserts `.validate()` returned a value; `to_internal_value` stores each `validate_<field>()` return value; `raise_errors_on_nested_writes`; `include_extra_kwargs` skips declared fields |
| DRF issue #3533 "Meta.read_only_fields does not affect declared fields" | https://github.com/encode/django-rest-framework/issues/3533 | maintainers closed it as intended: the silent ignore is permanent, so only a check can catch it |
| DRF issue #3460 "Document that explicitly declared fields will not have their attributes overridden by read_only_fields" | https://github.com/encode/django-rest-framework/issues/3460 | the same trap, reported again, answered with a docs note |
| Stack Overflow "has_object_permission not called" | https://stackoverflow.com/questions/54783424/has-object-permission-not-called | the recurring surprise that object permissions do not run outside `get_object()` |
| Stack Overflow "Django DRF: read_only_fields not working properly" | https://stackoverflow.com/questions/68946823/django-drf-read-only-fields-not-working-properly | the declared-field trap seen in the wild |
| Stack Overflow "Serializer validate data return None" | https://stackoverflow.com/questions/65420838/django-rest-framework-serializer-validate-data-return-none | a validate hook without `return` empties the data |
| OWASP Django REST Framework Cheat Sheet | https://cheatsheetseries.owasp.org/cheatsheets/Django_REST_Framework_Cheat_Sheet.html | change the `AllowAny` default; never use `exclude` or `"__all__"` on ModelSerializer (mass assignment, data exposure); without pagination "Denial of Service (DoS) problems or attacks could occur" |
| OWASP API Security Top 10 2023, API1 Broken Object Level Authorization | https://owasp.org/API-Security/editions/2023/en/0xa1-broken-object-level-authorization/ | the object-permission-skipped and unscoped-queryset hazards |
| OWASP API Security Top 10 2023, API3 Broken Object Property Level Authorization | https://owasp.org/API-Security/editions/2023/en/0xa3-broken-object-property-level-authorization/ | exposed and mass-assignable properties: the serializer field rules |
| OWASP API Security Top 10 2023, API4 Unrestricted Resource Consumption | https://owasp.org/API-Security/editions/2023/en/0xa4-unrestricted-resource-consumption/ | "Number of records per page to return in a single request-response" must be limited: the pagination rules |
| TestDriven.io, "Permissions in Django REST Framework" | https://testdriven.io/blog/drf-permissions/ | an `APIView` that fetches with `get_object_or_404` must call `self.check_object_permissions(request, obj)` itself |
| TestDriven.io, "Effectively Using Django REST Framework Serializers" | https://testdriven.io/blog/drf-serializers/ | `validate_<field>` returns the value; `validate()` returns the data |
| Haki Benita, "Improve Serialization Performance in Django Rest Framework" | https://hakibenita.com/django-rest-framework-slow | serializer cost is dominated by per-row database access first (`select_related`/`prefetch_related`) |
| Django docs, QuerySet API ("When QuerySets are evaluated") | https://docs.djangoproject.com/en/5.2/ref/models/querysets/ | which calls run a query on a related manager even after `prefetch_related` (`filter`, `exclude`, `order_by`, `aggregate`, …) |
| Semgrep registry, `python/django/security/audit/django-rest-framework/` | https://github.com/semgrep/semgrep-rules/tree/develop/python/django/security/audit/django-rest-framework | its one DRF rule is `missing-throttle-config`: no overlap with the build list |

Linters checked for overlap: Ruff's DJ family (DJ006 `exclude` and DJ007 `"__all__"` check Django **ModelForm** only, not
ModelSerializer; the `python` pack does not port DJ at all), Ruff S/B/PERF/RUF as ported in `packs/python/pack.json`
(nothing DRF-aware), Bandit (no DRF checks), pylint-django (DRF transforms only, no DRF checks), flake8-django (ModelForm
only), DRF's own system checks (`rest_framework.W001`: `PAGE_SIZE` without `DEFAULT_PAGINATION_CLASS`; label-studio
silences it), and DRF's runtime assertions (ModelSerializer without `fields`/`exclude`, nested writes without
`create()`/`update()`, `validate()` returning None).

## 2. Candidates

Corpus: the 8 DRF repositories of `~/rerail-bench/python-arch/corpus` (paperless-ngx, label-studio, plane, InvenTree,
pretix, addons-server, DefectDojo, linkding) plus four DRF apps cloned for this pack at pinned commits (netbox,
Tandoor recipes, wger, babybuddy; appended to `MANIFEST.tsv`). Probe counts are text or Python-`ast` searches over
non-test code, before any rule existed; the writers' engine runs give the real numbers.

| Id | Catches | Why it is a bug | Sources | Cross-file? | Linter overlap | Engine detection | Verdict |
|---|---|---|---|---|---|---|---|
| object-permission-skipped | a DRF view whose permission classes (its own, a base's, or the settings default) implement `has_object_permission`, where a handler fetches the row itself (`get_object_or_404`, `M.objects.get`) and never calls `self.get_object()` or `self.check_object_permissions()` | the object check never runs: any user who passes `has_permission` reads or changes any row (BOLA) | DRF Permissions; TestDriven permissions; OWASP API1; SO 54783424 | yes: permission classes in another module, settings default | none | permission classes resolved through imports and `REST_FRAMEWORK` settings; handler body fetch vs `get_object`/`check_object_permissions` calls | **build**: label-studio (default `HasObjectPermission`), addons-server, DefectDojo, Tandoor all use object permissions with hand-written handlers |
| view-without-permission | a DRF view or `@api_view` with no `permission_classes` (own or inherited) while the project's `DEFAULT_PERMISSION_CLASSES` is unset or `AllowAny` | DRF's default is `AllowAny`: the endpoint is public | DRF Permissions; DRF Settings; OWASP DRF cheat sheet | yes: settings file | Semgrep has none (throttle only) | settings `REST_FRAMEWORK` dict read in any file; view class hierarchy | **build**: paperless-ngx sets no default permission class |
| serializer-exposes-secret-field | a ModelSerializer whose output includes a model field holding a secret (password hash, token, API key, secret, private key, OTP seed) through `"__all__"`, `exclude`, or the field listed, with no `write_only` | the secret goes out in every response (OWASP API3) | DRF Serializers ("unintentionally exposing data"); OWASP DRF cheat sheet; OWASP API3; DRF 3.3 announcement | yes: the model's fields (`Meta.model`, another module, bases) | Ruff DJ007/DJ006: ModelForm only | `Meta.model` resolved to the model class; its fields by name; `fields`/`exclude`/`extra_kwargs`/declared fields | **build**: 74 `"__all__"` and 26 `exclude` lines in serializer modules of the 12 repositories (non-test); which reach secret fields is the writer's measurement |
| declared-field-meta-ignored | `Meta.read_only_fields` or `Meta.extra_kwargs` naming a field the serializer (or a base serializer) declares explicitly | DRF silently ignores both for declared fields: a field meant read-only stays writable (mass assignment), a `write_only` password stays readable | DRF Serializers (two notes); DRF issues #3533, #3460; SO 68946823 | yes when the field is declared on a base serializer | none | Meta options vs class-body field assignments, through the run's serializer bases | **build**: 23 `read_only_fields` and 3 `extra_kwargs` hits on declared fields (e.g. InvenTree `users/serializers.py:214` lists `username` read-only while declaring it a writable CharField) |
| validate-returns-value | a serializer's `validate_<field>()` that can end without returning (or with a bare `return`), and `validate()` likewise | DRF stores the return value: the field becomes None silently; `validate()` without a value fails an assertion on every request | DRF Serializers; DRF source; TestDriven serializers; SO 65420838 | yes: serializer bases | none (the django pack's clean-field-returns-value covers Django forms only) | method of a class deriving from a DRF serializer; flow to the end of the body | **build**: candidates in label-studio `projects/serializers.py:583`, `users/product_tours/serializers.py:61`, plane `page.py:205`, addons-server `accounts/serializers.py:169` |
| related-field-not-prefetched | a list view (ListAPIView, ModelViewSet, …) whose serializer reads a relation of each row (a nested serializer on a ForeignKey/OneToOne/M2M field, `source="fk.attr"`, `depth`) while the view's queryset never `select_related`s/`prefetch_related`s it | one extra query per row of every list response (N+1) | DRF Generic views ("Avoiding N+1 Queries"); Haki Benita; Django querysets docs | yes: view → serializer → model, three files | none; django's select-related-in-loop covers explicit loops only | view's `serializer_class` and `queryset`/`get_queryset` chain; serializer's fields; model's relation fields | **build** |
| method-field-query | a `SerializerMethodField`'s `get_<x>(self, obj)` that runs a query no prefetch can serve (`M.objects…`, `obj.<rel>.filter/exclude/order_by/aggregate/…`) in a serializer used for lists | one query per row; the fix is `annotate()` or `Prefetch(to_attr=…)` in the view | DRF Generic views; Haki Benita; Django querysets docs | yes: serializer used by a list view or with `many=True` elsewhere | none | `get_<x>` methods paired with `SerializerMethodField` declarations; query calls in them; serializer use sites | **build**: 75 non-test files of the 12 repositories use SerializerMethodField |
| page-size-uncapped | pagination that lets the client pick the page size with no cap: the built-in `LimitOffsetPagination` (max_limit None) as default or view class, a subclass without `max_limit`, a PageNumberPagination subclass with `page_size_query_param` and no `max_page_size` | `?limit=100000000` loads the whole table in one response (OWASP API4) | DRF Pagination; OWASP DRF cheat sheet; OWASP API4 | yes: settings string names the class | none | settings `DEFAULT_PAGINATION_CLASS` value; `pagination_class`; pagination subclasses' attributes | **build**: InvenTree, DefectDojo, linkding, babybuddy set the built-in LimitOffsetPagination as default; label-studio `organizations/api.py:88` has `page_size_query_param` without a cap |
| list-without-pagination | a list endpoint (ListAPIView, ListModelMixin, ModelViewSet) with no `pagination_class` in its hierarchy while the project sets no `DEFAULT_PAGINATION_CLASS` | every row of the table in one response, growing with the data | DRF Pagination; OWASP DRF cheat sheet; OWASP API4; DRF check W001 | yes: settings | DRF's W001 only fires when `PAGE_SIZE` is set without a class | settings + view hierarchy | **build** (expect `log`: some lists are small by nature): label-studio, paperless-ngx, plane, Tandoor set no default class |
| queryset-evaluated-once | `self.queryset` iterated or returned unchanged in a view method, or a class-level `queryset` that calls `now()` or is evaluated at import | results cached on the class are served to every later request | DRF Generic views (two warnings) | no | none | class attribute reads | **drop**: zero occurrences in 12 DRF repos (only `self.queryset.get(...)`, which does not cache) |
| request-data-without-serializer | `M.objects.create(**request.data)`, `obj.update(**request.data)`, setattr loops over `request.data` | mass assignment without validation | OWASP API3; DRF Serializers | no | none | call with `**request.data` into ORM | **drop**: zero occurrences; every `**request.data` in the corpus feeds a serializer |
| csrf-exempt-session-auth | a `SessionAuthentication` subclass whose `enforce_csrf` returns without checking | disables CSRF for cookie-authenticated API calls | DRF Authentication; DRF AJAX/CSRF topic | no | none | method override body | **drop**: the 2 overrides in the corpus (plane, pretix) both perform a real origin/CSRF check; no evidence teams in this corpus hit it |
| is-valid-ignored | `serializer.is_valid()` as a bare statement followed by `.save()`/`.validated_data` | a 500 (AssertionError) instead of a 400 | DRF Serializers | no | none | statement shape | **drop**: zero non-test occurrences on serializers (22 bare `is_valid()` are Django forms or tests) |
| writable-nested-no-create | a ModelSerializer with a writable nested serializer field and no `create()`/`update()` | DRF raises an AssertionError on the first such write | DRF Serializers | yes | DRF's runtime assertion | class fields and methods | **drop**: DRF already fails loudly at the first write; 338 probe hits are dominated by plain Serializers and read-only uses |
| allowany-on-write | `permission_classes = [AllowAny]` on a view with write actions | anonymous writes | OWASP DRF cheat sheet | no | none | attribute + handler names | **drop**: all 30 corpus uses (plane auth, invite, public "space" views) are public by design; one file cannot tell intent |
| get-queryset-unscoped | `get_queryset()`/`queryset` of a model with an owner FK not filtered by `request.user` | BOLA on list/detail | DRF Filtering; OWASP API1 | yes | none | model owner FK + queryset filters | **drop**: tenancy is scoped through workspaces, projects and filter backends in every corpus app; the matcher cannot see it, so matches would be mostly harmless. object-permission-skipped covers the precise subset |
| object-only-permission-on-list | a list/create view whose only permission classes implement `has_object_permission` and not `has_permission` | list and create run unchecked | DRF Permissions | yes | none | permission classes' methods | **drop**: zero occurrences; every corpus object-permission class is paired with an authenticated `has_permission` |
| owner-from-request-body | a ModelSerializer exposing a writable `owner`/`user`/`created_by` FK on a create view without `perform_create` setting it | a client assigns rows to another user | DRF Generic views (`perform_create`); OWASP API3 | yes | none | model FK + serializer + view | **drop**: ~40 probe hits, mostly owners that are assignable by design (paperless owner sharing, netbox/InvenTree `Owner` objects); cannot tell intent |
| throttle-missing | no `DEFAULT_THROTTLE_CLASSES` | brute force | OWASP DRF cheat sheet; Semgrep | yes | Semgrep `missing-throttle-config` | settings | **drop**: Semgrep covers it, and DRF documents throttling as no security control |
| pagination-class-none | `pagination_class = None` on a list view | unbounded list | DRF Pagination | no | none | attribute | **drop**: 30 corpus uses, all deliberate on small collections (addons-server comments say so); list-without-pagination keeps the unbounded case where no class applies at all |

## 3. Build list, ranked

| Rank | Rule id | Risk | Why this rank |
|---|---|---|---|
| 1 | `drf-practice-object-permission-skipped` | security | any authenticated user reads or changes any row; silent |
| 2 | `drf-practice-view-without-permission` | security | a public endpoint by default; silent |
| 3 | `drf-practice-serializer-exposes-secret-field` | security | secrets in every response |
| 4 | `drf-practice-declared-field-meta-ignored` | security | a field believed read-only stays writable (or a write-only one readable); silent |
| 5 | `drf-practice-validate-returns-value` | correctness | the field's value silently becomes None |
| 6 | `drf-practice-page-size-uncapped` | availability | one request loads a whole table |
| 7 | `drf-practice-list-without-pagination` | availability | every request loads a whole table, growing with the data |
| 8 | `drf-practice-related-field-not-prefetched` | performance | one query per row on list endpoints |
| 9 | `drf-practice-method-field-query` | performance | one query per row on list endpoints |

## 4. The library

`rules/lib/drf.rerail` is the pack's vocabulary (its interface comment lists every relation): DRF classes through
the run's class hierarchy and known third-party bases (`drf_lib`, `drf_ext_base`), views and their handlers,
serializers and their Meta options as Python inherits them (`drf_opt`, `class Meta(Base.Meta)` included),
declared fields, permission classes (`drf_effective_perm_q`: a view's own or inherited `permission_classes`, else the
settings default), settings (`REST_FRAMEWORK` as a dict, `dict(...)`, subscript assignment or `.update()`), and
`select_related`/`prefetch_related` loads. What it says about a class it says by symbol (qualified names, and the
values a class's attributes hold by their home, `q.x`), so no node crosses files (decision 0691); a rule reads the
answers back at its own file's nodes in its findings. It includes `rules/lib/django.rerail`, a verbatim copy of the django pack's
library (a pack's rules can include only core's library and files beside them), for models, querysets and test files.

Proven by a probe rule (one finding per relation) on a case repository of six files (settings, models, permissions,
serializers with an inherited Meta and a `Base.Meta.fields + [...]` list, a base view in another module, a viewset,
an `@action` and an `@api_view`): every relation found what it claims. On label-studio it resolved the settings
default `core.api_permissions.HasObjectPermission` (a string relative to the project's import root) to the run's
class and found it as the effective object permission of 82 views; the run showed that label-studio's
`rest_flex_fields.FlexFieldsModelSerializer` and Tandoor's `WritableNestedModelSerializer` serializers were missed,
which `drf_ext_base` now covers.
