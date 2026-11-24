# Rerail rule pack: Django REST framework

Rerail's own Django REST framework rules, from the DRF docs and maintainers' review advice, for safe serializers, permissions, querysets and pagination.

This pack is private while its rules are written and reviewed: it is not published to npm, and is installed from
a checkout with `rerail packs add packs/drf`.

## Rules

| Rule | Mode | Catches |
|---|---|---|
| `drf-practice-declared-field-meta-ignored` | Steer | DRF practice: read_only_fields or extra_kwargs naming a declared serializer field |
| `drf-practice-list-without-pagination` | Log | DRF practice: list endpoint over a model table with no pagination class anywhere |
| `drf-practice-object-permission-skipped` | Steer | DRF practice: a handler that fetches its row itself, skipping the view's object permission |
| `drf-practice-page-size-uncapped` | Steer | DRF practice: pagination that lets the client choose the page size with no upper bound |
| `drf-practice-related-field-not-prefetched` | Log | DRF practice: a list view's serializer reads a relation its queryset does not load |
| `drf-practice-serializer-exposes-secret-field` | Log | DRF practice: a ModelSerializer that returns a password, token or API key |
| `drf-practice-validate-returns-value` | Steer | DRF practice: serializer validate_<field>() or validate() that can end without returning |
| `drf-practice-view-without-permission` | Steer | DRF practice: a view that sets no permission classes in a project whose default permission is AllowAny |

## Rule details

### `drf-practice-declared-field-meta-ignored`

Rerail's own rule, not a port. ModelSerializer applies `Meta.read_only_fields` and `Meta.extra_kwargs` only to the
fields it builds from the model. A field the serializer declares itself (`username = serializers.CharField()`), in its
own body or in a base serializer, ignores both options, and DRF says nothing. A field listed read-only stays writable,
so any client can set it (OWASP API3, mass assignment). `extra_kwargs = {"password": {"write_only": True}}` on a
declared `password` leaves the password in every response.

The rule matches a ModelSerializer or HyperlinkedModelSerializer outside test code when all of these hold:

- it declares field `x` as a field call: a rest_framework field or a subclass, a nested serializer of the run, or a
  `…Field`/`…Serializer` class from another package. The declaration is its own, or the nearest base serializer's in
  any module of the run, when no class in between reassigns `x` (`x = None` removes the field).
- its Meta names `x`. The Meta can be its own, inherited through `class Meta(Base.Meta)`, or its base's when it has
  no Meta of its own. The name counts in `read_only_fields` (a list or tuple, `+`, `*splat` or
  `Other.Meta.read_only_fields`) or as a key of `extra_kwargs` (a dict literal or `dict(...)`).

Each serializer and field is reported once:

- at the name in the Meta option when the name is written in the serializer's own Meta;
- otherwise at the field's declaration when the serializer declares the field;
- otherwise (both inherited) at the class name, unless a base serializer already has the same name and declaration.
  That base is reported, and its subclasses are not.

The fix starts at the reported place. The message for `read_only_fields` says to set `read_only=True` on the declared
field. The message for `extra_kwargs` says to pass those arguments to the declared field.

It leaves alone:

- `read_only_fields` entries for fields that are read-only anyway: `read_only=True`, or a SerializerMethodField,
  ReadOnlyField, HiddenField, HyperlinkedIdentityField, StringRelatedField or a subclass of one;
- `extra_kwargs` entries whose every option the declared call already passes with the same text. `read_only: True` on
  a read-only field also counts as passed, and so does `required: False` on a field with a `default=` (label-studio's
  `created_by`). An empty entry is left alone too;
- a serializer whose own or a run base's `__init__` or `get_fields` reads the option by name. Such a mixin applies the
  option to declared fields itself (addons-server's version serializer sets `fields[name].read_only` from
  `Meta.read_only_fields`);
- for `read_only_fields`, a serializer that refuses all input: its own or a run base's `validate`,
  `to_internal_value` or `run_validation` raises unconditionally, as pretix's `OrderPositionSerializer` does;
- a run class that is not a DRF field (InvenTree's `OptionalField` placeholder), plain Serializers, fields named in
  Meta but not declared (DRF honours both options for those), and test code.

A field named in both options is reported for `read_only_fields` only.

**Gaps:**

- With a diamond of base serializers, the rule takes the nearest declaration along each path instead of DRF's MRO.
- A project field class whose own `__init__` forces `read_only` is reported.
- `extra_kwargs` built by a call other than `dict()`, by a `**` splat or by a comprehension is not read.
- A base serializer that the run resolves only under another import root is not followed, for example label-studio's
  `from core... import` of `label_studio/core/...`.

**Corpus** (commits from `MANIFEST.tsv`; every run through `corpus-run`, the final rule):

| Repository | Commit | Matches | Read | Real |
|---|---|--:|--:|--:|
| sissbruecker/linkding | `27b7303b` | 1 | 1 | 1 |
| babybuddy/babybuddy | `1638cfb0` | 0 | 0 | 0 |
| TandoorRecipes/recipes | `22b5841a` | 1 | 1 | 1 |
| paperless-ngx/paperless-ngx | `faa6f09f` | 0 | 0 | 0 |
| wger-project/wger | `f903c66d` | 0 | 0 | 0 |
| HumanSignal/label-studio | `2f42caec` | 0 | 0 | 0 |
| inventree/InvenTree | `939df605` | 6 | 6 | 6 |
| makeplane/plane | `723bb178` | 0 | 0 | 0 |
| mozilla/addons-server | `7e3374fe` | 0 | 0 | 0 |
| DefectDojo/django-DefectDojo | `cd0a4483` | 0 | 0 | 0 |
PRETIX_NETBOX_ROWS

PRECISION_LINE

Examples of real matches:

- InvenTree `src/backend/InvenTree/users/serializers.py:607`: `UserCreateSerializer` lists `is_staff` and
  `is_superuser` in its read_only_fields to "prevent creation of users with superuser or staff permissions". Both are
  writable BooleanFields declared on `ExtendedUserSerializer`, so a user with the admin role's `add` permission can
  create a superuser. Two matches.
- InvenTree `users/serializers.py:214`: `UserSerializer` lists `username` and `email` read-only and declares both as
  writable fields. The same file's `MeUserSerializer` already re-declares them with `read_only=True`, with the comment
  "Meta.read_only_fields has no effect on explicitly declared fields".
- InvenTree `part/serializers.py:577`: `creation_user` is a writable `PrimaryKeyRelatedField`, listed read-only, so a
  client can set who created the part. InvenTree `common/serializers.py:860` has the same shape for `filename`.
- Tandoor `cookbook/serializer.py:843`: `RecipeImportSerializer` (WritableNestedModelSerializer) lists `storage`
  read-only, and `storage` is a writable nested `StorageSerializer`, so a client can create or change storage backends
  through a recipe import.
- linkding `bookmarks/api/serializers.py:90`: `tag_names` is listed read-only, but `create()`/`update()` read it from
  `validated_data`. The entry is dead and misleading, and the message's alternative applies: remove it from
  read_only_fields because writes are intended.

A hand probe (Python `ast`, under Python 3.9, which could not parse Tandoor's serializer module) listed each
serializer's own declared fields named in its own Meta in the twelve repositories. Outside the exemptions above it found
nothing the rule misses. Its other candidates were fields that are read-only anyway, InvenTree's `StockItemSerializer`
`extra_kwargs` repeating `write_only=True`, an `OptionalField` placeholder, and label-studio's `created_by` with a
default. addons-server and DefectDojo ran on an earlier revision of the rule, and the later changes only narrow it.

**Sources:**
- DRF API guide, [Serializers](https://www.django-rest-framework.org/api-guide/serializers/): "if a field has already
  been explicitly declared on the serializer class (or a parent class), then the read_only_fields option will not apply
  to that field. Set read_only=True on the field itself instead"; "if the field has already been explicitly declared on
  the serializer class, then the extra_kwargs option will be ignored".
- DRF issue [#3533](https://github.com/encode/django-rest-framework/issues/3533), closed as intended, and
  [#3460](https://github.com/encode/django-rest-framework/issues/3460), answered with the docs note.
- Stack Overflow, [read_only_fields not working properly](https://stackoverflow.com/questions/68946823/django-drf-read-only-fields-not-working-properly).
- OWASP API Security Top 10 2023, [API3 Broken Object Property Level Authorization](https://owasp.org/API-Security/editions/2023/en/0xa3-broken-object-property-level-authorization/)
  (mass assignment).

### `drf-practice-list-without-pagination`

Paginate Django REST framework list endpoints over model tables when the project sets no default pagination class, so a response cannot grow with the table.

Rerail's own rule, not a port. DRF paginates nothing by default: DEFAULT_PAGINATION_CLASS and PAGE_SIZE are both
None, and a list endpoint with no pagination class serialises every row its queryset holds on every request. The
rule reads the project's REST_FRAMEWORK settings in every non-test file of the run (base, production and local
settings modules alike) and, when none of them names a DEFAULT_PAGINATION_CLASS (absent, or None), reports at the
class name:
- a concrete list view (lib/drf's drf_list_view: ListAPIView, ListCreateAPIView, ModelViewSet,
ReadOnlyModelViewSet, ListModelMixin) outside test code: no class of the run outside test code derives from it,
and its name does not start with `Base` or `Abstract`;
- that assigns no pagination_class, itself or through a base view of the run in any module, and neither it nor a
run base overrides paginate_queryset() or paginator;
- whose rows come from a model table: its (inherited) `queryset` is a queryset chain from a manager
(`Order.objects.filter(...)`), its serializer's Meta.model is a model of the run (drf_view_model), or a return
of its get_queryset() is such a chain, directly or through a local variable assigned one.
# settings.py: REST_FRAMEWORK = {"PAGE_SIZE": 50}                   no DEFAULT_PAGINATION_CLASS
class OrderViewSet(viewsets.ModelViewSet):                         reported: every order in one response
queryset = Order.objects.select_related("customer")
class CountryList(generics.ListAPIView):                           not reported: unpaginated on purpose
queryset = Country.objects.all()
pagination_class = None
It leaves alone: `pagination_class = None` and any other value, own or inherited, and any class attribute whose
name speaks of pagination (Tandoor's `pagination_disabled = True`); a view (or run base) that overrides
paginate_queryset() or paginator, or whose list(), get(), get_queryset() or filter_queryset() calls a function or
method named like a paginator (`self.paginate_queryset(...)`, label-studio's `paginator(tasks, request)`); a view
whose own (or a run base's) list() or get() calls neither `super().list(...)`, `super().get(...)` nor
`self.list(...)` (it returns something else; a ViewSet whose list() it writes itself); a viewset routed only
through `as_view({...})` maps that never map a method to "list" and registered with no router (plane's
UserEndpoint); a queryset attribute that is sliced (`Order.objects.all()[:50]`), and a get_queryset() that slices
anything; views whose rows the rule cannot tie to a model (a get_queryset() returning a list, a plain Serializer);
projects whose settings name any DEFAULT_PAGINATION_CLASS but None in any non-test module, or build REST_FRAMEWORK
from something but a dict literal or dict(...) (`{**BASE, ...}`, a function call); runs with no REST_FRAMEWORK
assignment (a reusable app: the host project's settings decide); test code; classes named ListAPIView in other
packages.
Gaps: a reusable app's list views are not reported (no settings in the run); a list() written from scratch that
serialises a whole queryset with `many=True` is left alone; whether a table grows with use (predictions, comments,
uploads) or stays small by nature (a project's webhooks, storages, saved views; a singleton config row) is not
something the rule can read, so it stays in log mode; a list view whose URL is never routed at all is reported; a
pagination class set by a third-party base view or in `__init__` is not seen; a get_queryset() that returns
`super().get_queryset()...` or `self.queryset...` counts through the view's queryset or serializer model only.

### `drf-practice-object-permission-skipped`

Rerail's own rule, not a port. Django REST framework runs a permission class's `has_object_permission` only inside
`get_object()` and `check_object_permissions(request, obj)`. A handler that fetches the row it acts on itself
(`get_object_or_404(Book, pk=pk)`, `Book.objects.get(pk=pk)`) and calls neither lets any user who passes the
view-level `has_permission` (usually: any signed-in user) read, change or delete any row by its id: OWASP API1,
broken object level authorization.

It reports the fetch call when all of these hold:

- **The handler's permission classes check objects.** A permission class checks objects when its
  `has_object_permission` (its own or a base's) reads its `obj` argument, or it is (derives from)
  `DjangoObjectPermissions`. The classes are the view's `permission_classes`, its own or a base view's in another
  module (also `api_settings.DEFAULT_PERMISSION_CLASSES + [...]`), else the settings' `DEFAULT_PERMISSION_CLASSES`,
  which may name the class by a dotted string. `@action(permission_classes=[...])` replaces them for its handler. An
  `@api_view` function uses its `@permission_classes([...])`, else the settings default.
- **The handler acts on one row:** `get`, `put`, `patch`, `delete`, `retrieve`, `update`, `partial_update`,
  `destroy`, an `@action(detail=True)`, or an `@api_view` function.
- **The fetch reads the view's rows:** `get_object_or_404(M, …)`, `get_object_or_404(<queryset>, …)`,
  `<queryset>.get(…)` or `<queryset>.filter(…).first()`, where the rows are the view's model (its `queryset`'s, or its
  serializer's `Meta.model`) or come from `self.get_queryset()`/`self.queryset`. In a view without a model and in a
  function view: a fetch of any model whose lookup reads a URL kwarg (a handler parameter other than `self`,
  `request` and `format`, or `self.kwargs`).
- **Nothing checks the row:** neither the handler nor a method of the view it calls through `self.<name>(…)` calls
  `get_object()`, `check_object_permissions()`, `has_object_permission()`, a DRF detail handler of the view
  (`super().update()`, `self.retrieve()` of a mixin, which call `get_object()`), or a permission function given the
  requester (`project.has_permission(request.user)`, `request.user.has_perm("change_book", book)`,
  `has_perms_owner_aware(user, …, doc)` with `user = request.user`), and the handler does not refuse requests itself
  (raises `PermissionDenied`, answers 403 or `HttpResponseForbidden`).

A function view gets its own message: DRF never runs `has_object_permission` for one, so the fix is an explicit
check or a lookup limited to the requester.

It leaves alone: lookups that pass `request.user`, an attribute of it (`organization=request.user.active_organization`)
or a name bound to one; views whose `get_queryset()` reads `request.user`, or a method of which (NetBox's
`initial()`) sets `self.queryset` from an expression reading it; list and create handlers; views whose permission
classes check no objects, or whose `has_object_permission` never reads the object (Tandoor's `CustomIsGuest`); plain
Django views; test files.

Model and serializer names resolve by suffix when the project's import root is not the run's (label-studio imports
`from tasks.models import Task` for `label_studio/tasks/models.py`; lib/drf's `drf_view_model` misses those).

**Gaps:** a fetch of another model than the view's in a view with a model (a project row fetched to authorise a task
row) is not reported; a fetch in a helper the handler calls is not reported (only the handler's own body is read);
a check in a module-level helper is seen only when its name says permission (`perm`, `access`, `allow`, `authori`)
and it is given the requester; `get_permissions()` overrides and a `permission_classes` property are not read; a
lookup scoped by tenant (`space=request.space`) counts as unscoped, and a `self.model.objects` root (a mixin's model
attribute) is not resolved.

**Sources:** DRF API guide, [Permissions](https://www.django-rest-framework.org/api-guide/permissions/) ("Object
level permissions are run by REST framework's generic views when `.get_object()` is called"; "the view code should
explicitly call `.check_object_permissions(request, obj)` … Function-based views will need to check object
permissions explicitly"); TestDriven.io, [Permissions in Django REST Framework](https://testdriven.io/blog/drf-permissions/)
(the `get_object_or_404` + `check_object_permissions` example); OWASP API Security Top 10 2023,
[API1 Broken Object Level Authorization](https://owasp.org/API-Security/editions/2023/en/0xa1-broken-object-level-authorization/);
Stack Overflow, [has_object_permission not called](https://stackoverflow.com/questions/54783424/has-object-permission-not-called).
No linter models DRF permissions; the django pack has no permission rule.

**Corpus** (`~/rerail-bench/python-arch/corpus`, commits from `MANIFEST.tsv`; every number below comes from a
`corpus-run` run; two earlier direct engine runs, wger and paperless-ngx, gave the same 0 and were rerun):

| Repository | Commit | Matches | Read | Real |
|---|---|--:|--:|--:|
| `babybuddy__babybuddy` | `1638cfb02e` | 0 | 0 | 0 |
| `TandoorRecipes__recipes` | `22b5841a78` | 3 | 3 | 3 |
| `paperless-ngx__paperless-ngx` | `faa6f09f28` | 0 | 0 | 0 |
| `inventree__InvenTree` | `939df605f4` | 0 | 0 | 0 |
| `wger-project__wger` | `f903c66d3c` | 0 | 0 | 0 |
| `HumanSignal__label-studio` | `2f42caecbf` | 0 | 0 | 0 |
| `makeplane__plane` | `723bb178f9` | 0 | 0 | 0 |
| `netbox-community__netbox` | `6997809981` | 0 | 0 | 0 |
| `mozilla__addons-server` | `7e3374fe53` | 0 | 0 | 0 |
| `DefectDojo__django-DefectDojo` | `cd0a4483a2` | 0 | 0 | 0 |
| **Total** | | **3** | **3** | **3** (100%) |

The three matches are one mixin, Tandoor's `DeleteRelationMixing` (`cookbook/views/api.py:577`, `:607`, `:639`: the
`protecting`, `cascading` and `nulling` detail actions). Each fetches `self.queryset.filter(pk=pk,
space=request.space).first()` and never runs the view's object permission; `RecipeBookViewSet` (`CustomIsOwner |
(CustomIsShared & IsReadOnlyDRF)`) and `RecipeViewSet` (`CustomRecipePermission`) mix it in, so any member of a space
can list the objects that protect, cascade from or point to another member's private recipe book or recipe, by id.

A diagnostic copy of the rule, run through `corpus-run` on four repositories, reported each fetch the rule reaches
and why it stays silent: label-studio 89 views with an object permission, 4 fetches silenced by a check
(`tasks/api.py:571` and `:754` call `task.project.has_permission(request.user)`, `ml/api.py:420`,
`organizations/api.py:311`) and 11 that fetch rows other than the view's (mostly projects looked up from a query
parameter, followed by `check_object_permissions`); paperless-ngx 20 views, 7 fetches silenced by
`has_perms_owner_aware(request.user, …)` with `HttpResponseForbidden`, 3 by another model; Tandoor 22 views, its three
`@api_view` functions that call `CustomRecipePermission().has_object_permission(...)` themselves, `UserSpaceViewSet.destroy`
(`super().destroy()`); wger 43 views and no fetch of a row in their handlers. NetBox's API viewsets fetch from
`self.queryset`, which `BaseViewSet.initial()` restricts by `request.user`; `DataSourceViewSet.sync` checks
`request.user.has_perm(...)`.

Proposed mode: `steer` (3 of 3 read matches real). The sample is small: the corpus apps mostly use `get_object()`,
which is the point of the rule; it is meant to catch new handlers that do not.

### `drf-practice-page-size-uncapped`

Rerail's own rule, not a port. DRF's paginators take the page size from the query string when the class names a
parameter for it, and cap it only by `max_limit` (`LimitOffsetPagination`) or `max_page_size`
(`PageNumberPagination`, `CursorPagination`). Both default to `None`, so `?limit=100000000` or
`?page_size=100000000` makes one request load and serialise every row of the table. The rule reads the project's
`REST_FRAMEWORK` settings, its views and its pagination classes across files and reports, outside test code:

- the built-in `rest_framework.pagination.LimitOffsetPagination` named as `DEFAULT_PAGINATION_CLASS` (a dotted
  string or a class reference, reported at the value), or as a pagination class: a class body's
  `pagination_class = ...`, `self.pagination_class = ...`, `@action(pagination_class=...)` (reported at the value);
- a project class deriving from `LimitOffsetPagination`, directly or through the run's classes in any module, with
  no `max_limit` of its own or of a run base (`max_limit = None` counts as none) and no capping `get_limit()`
  (reported at the class name);
- a project class deriving from `PageNumberPagination` or `CursorPagination` whose own or inherited
  `page_size_query_param` is a string, with no `max_page_size` and no capping `get_page_size()` (reported at the
  class name).

It leaves alone: a `max_limit`/`max_page_size` set to any value other than `None`, however large, assigned in
`__init__` (`self.max_limit = ...`) or defined as a property; a `get_limit()`/`get_page_size()` override, its own or a
run base's (netbox's `NetBoxPagination` caps in `get_limit()`), unless it hands the client's value back unchecked: a
`return super().get_page_size(request)` in a method that neither raises nor calls `min()`;
`limit_query_param = None` and `page_size_query_param = None`; `PageNumberPagination` and `CursorPagination` without
`page_size_query_param` (the client cannot change the size), built-in or subclassed; a setting or `pagination_class`
that names a project class (the class is judged where it is written); a project class used only as a base (a run class
derives from it, and no setting, `pagination_class` or call names it), whose subclasses are judged instead; test code.

**Gaps:** a cap read from configuration (`max_page_size = settings.TASK_API_PAGE_SIZE_MAX`) is trusted even when the
configured value is `None` or huge; a cap far above any real page is left alone (paperless-ngx's `StandardPagination`
has `max_page_size = 100000`); `page_size_query_param` set in `__init__` is not seen; an override that clamps through a
helper but also returns `super().get_page_size(...)` unchanged on some path is reported; pagination classes of
third-party packages are not followed.

**Corpus** (commits from `MANIFEST.tsv`):

| Repository | Commit | Matches | Read | Real |
|---|---|--:|--:|--:|
| sissbruecker/linkding | `27b7303b` | 1 | 1 | 1 |
| babybuddy/babybuddy | `1638cfb0` | 1 | 1 | 1 |
| TandoorRecipes/recipes | `22b5841a` | 0 | 0 | 0 |
| paperless-ngx/paperless-ngx | `faa6f09f` | 0 | 0 | 0 |
| inventree/InvenTree | `939df605` | 1 | 1 | 1 |
| wger-project/wger | `f903c66d` | 0 | 0 | 0 |
| HumanSignal/label-studio | `2f42caec` | 1 | 1 | 1 |
| pretix/pretix | `40ea9097` | 0 | 0 | 0 |
| mozilla/addons-server | `7e3374fe` | 0 | 0 | 0 |
| netbox-community/netbox | `69978099` | 1 | 1 | 1 |
| DefectDojo/django-DefectDojo | `cd0a4483` | 1 | 1 | 1 |

Precision 6 of 6 read matches. Examples: linkding `bookmarks/settings/base.py:151`, babybuddy
`babybuddy/settings/base.py:362`, InvenTree `src/backend/InvenTree/InvenTree/settings.py:615` and DefectDojo
`dojo/settings/settings.dist.py:856` set the built-in `LimitOffsetPagination` as the default for every list endpoint,
so an authenticated `?limit=100000000` serialises a whole table (bookmarks, child events, parts, findings).
label-studio `label_studio/organizations/api.py:86` (`OrganizationMemberListPagination`) takes `page_size` from the
client with no `max_page_size`; its `get_page_size()` turns `-1` into 1000000 and otherwise returns
`super().get_page_size(request)` unchanged. netbox `netbox/netbox/api/pagination.py:200`
(`LimitOffsetListPagination`, instantiated by the superuser-only background-task list views) has no `max_limit`;
its list is already in memory, so the cost is serialising all of it: real, low impact. The corpus's capped shapes
stay silent, and each shape is a case: wger's `WgerLimitOffsetPagination` (`max_limit = 999`, named by dotted string
in its settings) and `IngredientCursorPagination` (`max_page_size = 1000`), pretix's and addons-server's capped
`PageNumberPagination` subclasses, Tandoor's `DefaultPagination` (`max_page_size = 200`), label-studio's
`TaskPagination` (`max_page_size = settings.TASK_API_PAGE_SIZE_MAX`) and netbox's `NetBoxPagination` and
`StripCountAnnotationsPaginator` (capped in `get_limit()`).

**Sources:**
- DRF API guide, [Pagination](https://www.django-rest-framework.org/api-guide/pagination/): `max_limit` is "the
  maximum allowable limit that may be requested by the client. Defaults to `None`"; `max_page_size` is "the maximum
  allowable requested page size", "only valid if `page_size_query_param` is also set".
- OWASP [Django REST Framework Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Django_REST_Framework_Cheat_Sheet.html),
  `DEFAULT_PAGINATION_CLASS`: "Without proper pagination, Denial of Service (DoS) problems or attacks could occur if
  there's a lot of data."
- OWASP API Security Top 10 2023, [API4 Unrestricted Resource Consumption](https://owasp.org/API-Security/editions/2023/en/0xa4-unrestricted-resource-consumption/):
  an API is vulnerable when it sets no limit on the "Number of records per page to return in a single
  request-response".

### `drf-practice-related-field-not-prefetched`

Load every relation a DRF list view's serializer reads in the view's queryset (select or prefetch it), so a list request does not run one extra query per row.

Rerail's own rule, not a port. A list view serializes every row of its queryset; a serializer field that reads a
related object (or collection) of the row runs one query per row unless the queryset loaded it (DRF Generic views,
"Avoiding N+1 Queries"; Haki Benita, "Improve Serialization Performance in Django Rest Framework"). The view, the
serializer and the model usually sit in three modules, which no single-file check joins.
It matches a DRF list view (ListAPIView, ListCreateAPIView, ModelViewSet, ReadOnlyModelViewSet, ListModelMixin, or
a viewset's own `list`) outside test code whose (inherited) `serializer_class` is a ModelSerializer of the run with
Meta.model mc, when a field of that serializer reads relation r of mc for each row:
- a declared field that is a nested serializer on r (by its name or `source="r"`);
- a declared field with a dotted `source="r.attr…"`;
- a declared related or plain field on r that renders the related object (StringRelatedField, SlugRelatedField,
CharField(source="r"), ...): every rest_framework field but PrimaryKeyRelatedField and HyperlinkedRelatedField,
which read the key (`r_id`) without a query;
- any field on a "many" relation r (a ManyToManyField, a reverse ForeignKey by its related_name or `<model>_set`),
PrimaryKeyRelatedField(many=True) included: the related rows take a query each time;
- a ManyToManyField or reverse relation named in Meta.fields (a forward ManyToManyField also through `"__all__"` or
`exclude`), which ModelSerializer renders as a list of keys, one query per row;
- with `Meta.depth >= 1`, any forward relation in the fields, which ModelSerializer nests.
Relations: ForeignKey, OneToOneField, ManyToManyField, ParentalKey/ParentalManyToManyField, mptt's Tree* fields and
taggit's TaggableManager, declared on mc or a model base of the run; reverse ones from a relation of another model
whose target is mc (a class reference, "Model" or "app.Model"). Write-only fields (`write_only=True`, or in
Meta.extra_kwargs) read nothing and are skipped, as are SerializerMethodFields (drf-practice-method-field-query).
The queryset must be visible and load nothing for r: the view's (inherited) `queryset = ...` and every
`get_queryset()` it runs (its own, or a run base's when it has none, and the bases' when it calls super()) return
chains of QuerySet methods from `M.objects` (M = mc), `self.queryset`, `super().get_queryset()`, or local names
assigned only such chains; and no `select_related(...)`/`prefetch_related(...)` in the view's class body or its run
bases loads r: a lookup equal to r or starting with `r__`, `Prefetch("r")`, or, for a forward one-row relation,
`select_related()` with no argument.
Reported once per view, relation and serializer field, where the fix goes: at the view's own get_queryset() when it
has one; else at its own `queryset =` assignment; else (the queryset is inherited) at its own `serializer_class =`
assignment, the choice that brought in the reads; else at the class name, unless a run base view that lists with the
same serializer is reported instead.
class BookSerializer(serializers.ModelSerializer):
author = AuthorSerializer(read_only=True)
class Meta: model = Book; fields = ["id", "author", "tags"]   (tags: ManyToManyField)
class BookViewSet(viewsets.ModelViewSet):
queryset = Book.objects.all()                     reported twice: `author` and `tags` are not loaded
serializer_class = BookSerializer
class BookList(generics.ListAPIView):
queryset = Book.objects.select_related("author").prefetch_related("tags")   not reported
serializer_class = BookSerializer
Left alone: a plain ForeignKey in Meta.fields (PrimaryKeyRelatedField reads `author_id`); views whose queryset the
rule cannot see: a helper function, a custom QuerySet or manager method (`Book.objects.with_stats()`), a related
manager (`self.request.user.books`), a custom manager attribute, a model whose `objects` is a manager class of the
run with its own get_queryset (it may load the relation), a root model other than the serializer's; views with a
get_serializer_class() or get_serializer() of their own or a run base's (the list action may use another
serializer); serializers overriding to_representation (own or a run base's); fields not in Meta.fields; detail-only
views (RetrieveAPIView, APIView); test code.
Gaps: the relation is the source's first step only, so `source="author.country.name"` is satisfied by loading
`author`; a `Prefetch("r", to_attr="x")` counts as loading r; select_related() with no argument counts for nullable
ForeignKeys, which it skips; a string relation target resolves by class name and can pick a same-named model of
another app; nested serializers' own relations are not followed; `fields` built by a call is not read.

### `drf-practice-serializer-exposes-secret-field`

Rerail's own rule, not a port. A ModelSerializer returns every model field its `Meta` lets in: all of them with
`fields = "__all__"`, every one an `exclude` does not name, or the ones `fields` lists. When one of them holds a
secret, every response of every view using the serializer returns it (OWASP API3, Broken Object Property Level
Authorization). The rule resolves the serializer's `Meta.model` to a model of the run in any module, reads its fields,
its own and those it inherits from the run's model bases (an abstract base in a third module), and reports at the
`Meta` value that lets the secret in: the `"__all__"` string or the `exclude` value (once per secret field), or the
listed string in `fields` (lists joined with `+` and `Base.Meta.fields` included).

A field is a secret when its whole name says so and its class can hold one. Names: `[*_]password[s][_hash]`
(`passwd`, `passphrase`), `[*_]secret[s][_key|_access_key|_token|_value]`, `[*_]token[s][_key|_secret|_value|_data]`,
`[*_](api|access|secret|private|signing|encryption)[_]key[s]`, `[*_](otp|totp|hotp)[_secret|_seed|_key]`,
`[*_](mfa|2fa)_(secret|seed|key)`, `[*_]credential[s]`. Booleans, numbers, dates and times, durations, auto keys and
relations are left out, so `password_changed_at`, `token_expires`, `tokens_used`, `has_password` and a BooleanField
`secret_santa` never match. A model deriving from Django's `AbstractBaseUser`, `AbstractUser` or `User` (and
`get_user_model()`, or a module's `User = get_user_model()`) has Django's `password` hash. A model field class the
project defines (`EncryptedTextField`) or another package's (`AESField`) counts when its name ends in `Field`.
Names also resolve across a project's import root: label-studio imports `label_studio/organizations/models.py` as
`organizations.models`, so a model, model base or serializer base that no class of the run has by its imported name is
the class of that name whose qualified name ends with it.

It leaves alone a secret the serializer hides: a declaration of the name (in the serializer or a serializer base it
inherits it from) as `x = None`, a field with `write_only=True`, a `HiddenField` or `SerializerMethodField`, a field
with another `source=`, or a field class that is not rest_framework's own (paperless-ngx's `ObfuscatedPasswordField`);
with no declaration, `Meta.extra_kwargs = {"x": {"write_only": True}}`; and a serializer with a `to_representation`
(its own or a base's, a plain mixin included) whose class, or a base class, names the field as a string outside its
`Meta` (`data.pop("password")`, or label-studio's `secure_fields = [...]` list that `to_representation` pops in a
loop). Plain `Serializer`s, serializers whose model is outside the run (other than Django's users), and test code are
left alone.

**Gaps:** a token the endpoint that creates it returns once by design is reported (plane's `APITokenSerializer`); a
field named like a secret that holds an identifier is reported (plane's `ExporterHistory.token`); a write serializer
whose view always answers with another serializer is reported; a declared plain field next to `extra_kwargs`
write_only is reported (DRF ignores `extra_kwargs` for declared fields, so it is exposed); a secret under a name the
list misses (`key`, `pin`, `salt`) is not; `fields` built by a call or a comprehension (plane's
`[f.name for f in User._meta.fields if f.name != "password"]`) is not read; a `to_representation` that drops the
field through a helper in another module is reported.

**Corpus** (commits from `MANIFEST.tsv`; every run through `corpus-run`):

| Repository | Commit | Matches | Read | Real | One-time token | Not a secret |
|---|---|--:|--:|--:|--:|--:|
| HumanSignal/label-studio | `2f42caec` | 4 | 4 | 4 | 0 | 0 |
| makeplane/plane | `723bb178` | 4 | 4 | 2 | 1 | 1 |
| paperless-ngx/paperless-ngx | `faa6f09f` | 1 | 1 | 1 | 0 | 0 |
| netbox-community/netbox | `69978099` | 1 | 1 | 1 | 0 | 0 |
| DefectDojo/django-DefectDojo | `cd0a4483` | 2 | 2 | 2 | 0 | 0 |
| pretix/pretix | `40ea9097` | 3 | 3 | 0 | 2 | 1 |
| mozilla/addons-server | `7e3374fe` | 0 | 0 | 0 | 0 | 0 |
| inventree/InvenTree | `939df605` | 0 | 0 | 0 | 0 | 0 |
| TandoorRecipes/recipes | `22b5841a` | 0 | 0 | 0 | 0 | 0 |
| wger-project/wger | `f903c66d` | 0 | 0 | 0 | 0 | 0 |
| babybuddy/babybuddy | `1638cfb0` | 0 | 0 | 0 | 0 | 0 |
| sissbruecker/linkding | `27b7303b` | 0 | 0 | 0 | 0 | 0 |

pretix's 3 matches are not real: `api/views/device.py:140` returns `api_token` and `api/serializers/organizer.py:468`
returns `initialization_token`, both one-time device-provisioning tokens shown by design, and
`api/serializers/orderchange.py:255` `OrderPositionChangeSerializer` is input-only (its views never return its `.data`;
the "Not a secret" column counts it). pretix's other order and gift-card serializers derive from a non-DRF base and are
not seen.

Precision: 10 of 16 read matches (62.5%) return a stored secret to API clients; 3 more are one-time tokens returned by
the request that creates them (81% counting them): plane `apps/api/plane/app/serializers/api.py` `APITokenSerializer`
(its list and detail views use `APITokenReadSerializer`, which excludes `token`) and pretix's two device tokens. Of the
other 3, plane `ExporterHistory.token` is an export job's identifier, not a credential, and 2 are pretix's input-only
and by-design shapes. That precision puts the rule in `log`. Real matches include DefectDojo
`dojo/system_settings/api/serializer.py:9` (`SystemSettingsSerializer`, `fields = "__all__"`, returns
`jira_webhook_secret` and `slack_token` to superusers) and:

- label-studio `label_studio/webhooks/serializers_for_hooks.py` `ProjectWebhookSerializer` (`fields = '__all__'`): the
  project's `token` and `task_data_password` go out in every project webhook payload, to whatever URL the webhook names.
- label-studio `label_studio/projects/serializers.py` `ProjectSerializer` lists `task_data_password`, the credentials
  for the task data's storage, so `GET /api/projects/` returns it to every project reader.
- label-studio `label_studio/organizations/serializers.py` `OrganizationSerializer` (`'__all__'`): the organization's
  invite token.
- plane `apps/api/plane/app/serializers/workspace.py` and `project.py`: workspace and project invite tokens in the
  admins' invite lists.
- paperless-ngx `src/documents/serialisers.py` `WorkflowActionSerializer` lists `passwords` (the PDF passwords a
  workflow tries) in every workflow response.
- netbox `netbox/extras/api/serializers_/events.py` `WebhookSerializer` lists `secret`, the webhook's HMAC key.

Silent on purpose: plane's `WebhookSerializer` (`to_representation` pops `secret_key` unless the create view asks for
it), label-studio's S3, GCS and Redis storage serializers (popped in `to_representation`), babybuddy's
`WebhookEndpointSerializer` (`extra_kwargs` write_only), paperless-ngx's user, profile, mail account and settings
serializers (`ObfuscatedPasswordField`).

**Sources:**
- DRF API guide, [Serializers](https://www.django-rest-framework.org/api-guide/serializers/#specifying-which-fields-to-include):
  "It is strongly recommended that you explicitly set all fields that should be serialized using the `fields`
  attribute. This will make it less likely to result in unintentionally exposing data when your models change."
- DRF [3.3 announcement](https://www.django-rest-framework.org/community/3.3-announcement/): `fields` or `exclude`
  became mandatory on ModelSerializer so that new model fields are not exposed implicitly.
- OWASP [Django REST Framework Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Django_REST_Framework_Cheat_Sheet.html):
  "DO NOT use the `exclude` Meta property", and do not use `fields = "__all__"`.
- OWASP API Security Top 10 2023, [API3 Broken Object Property Level Authorization](https://owasp.org/API-Security/editions/2023/en/0xa3-broken-object-property-level-authorization/).
- Overlap: Ruff DJ006/DJ007 check Django `ModelForm` only (and the `python` pack does not port them); no DRF linter
  checks serializer output.

### `drf-practice-validate-returns-value`

Rerail's own rule, not a port. `Serializer.to_internal_value` stores whatever a field hook `validate_<field>(value)`
returns as the field's validated value. A path that falls off the end of the hook, or a bare `return`, turns the value
into None, and `save()` writes None. DRF raises no error. `Serializer.run_validation` asserts that `validate(attrs)`
returned something (".validate() should return the validated data"). A `validate()` that can end without a value
fails every request that takes that path with an AssertionError, a 500.

The rule matches a method in a class that derives from a rest_framework serializer, through the run's classes in any
file, outside test code:

- `validate_<x>(self, value)`, with exactly two positional parameters, when DRF runs it for `x`. That holds when the
  serializer declares `x`, itself or in a base serializer of the run, when its Meta.fields names `x` (its own Meta or
  an inherited one), or when `fields = "__all__"` and `x` is a field of its Meta.model in the run. A field of a run
  serializer deriving from the class counts too: a base serializer's `validate_slug` runs for the subclass that
  declares `slug`.
- `validate(self, attrs)`, with exactly two positional parameters.

It reports the method's name when a path can reach the end of the body, and each bare `return`. The flow analysis is
the one of the django pack's `django-practice-clean-field-returns-value`: `if`/`elif`/`else`, `try`/`except`/`else`/
`finally`, `with`, loops with `else` and `break`, `while True`, and `match` with or without a catch-all case.

It leaves alone:

- a method whose every path returns or raises, a body that only raises (`raise NotImplementedError`);
- `return None` written out, as a value chosen on purpose;
- a `validate_<x>` for an `x` that is not a field of the serializer. DRF never calls it, so it is a helper:
  paperless-ngx's `validate_parameters_remove_password`, DefectDojo's `validate_findings_not_accepted_elsewhere`,
  addons-server's `validate_name_for_scanner`;
- a `validate_<x>` that the class or one of its run bases calls itself (`self.validate_x(...)`);
- in a drf-writable-nested serializer, the hook of a `many=True` nested serializer field. That library pops the field
  from `validated_data` and saves the related rows from the request's initial data, so the hook's return value is never
  used (Tandoor's `RecipeSerializer.validate_steps`). A single nested field stays reported: a None there is saved;
- `validate_<x>` with any other signature, classes that are not serializers (a field's `validate_empty_values`), and
  test code.

**Gaps:**

- Fields added at run time (`self.fields["x"] = ...` in `__init__`), fields that come from `Meta.exclude`, and
  `"__all__"` with a model outside the run are not read, so their hooks are not checked.
- A serializer whose `to_internal_value` or `run_validation` skips the hooks is still checked.
- `return None` in `validate()` fails DRF's assertion too, but it is left alone as written on purpose.
- A hook on a mixin class that does not itself derive from a serializer is not checked.

**Overlap:** none in the DRF ecosystem. Ruff's RET503 (implicit return) is not among the python pack's ports
(`packs/python/pack.json` ports RET501, RET502 and RET505-RET508). RET503 would flag every function that returns a
value on some path, without knowing which methods DRF reads the value of. RET502 (in the python pack) flags a bare
`return` only in a function that also returns a value elsewhere, and says nothing about a hook that falls off the end.

**Corpus** (commits from `MANIFEST.tsv`; every run through `corpus-run`):

| Repository | Commit | Matches | Read | Real |
|---|---|--:|--:|--:|
| sissbruecker/linkding | `27b7303b` | 0 | 0 | 0 |
| babybuddy/babybuddy | `1638cfb0` | 0 | 0 | 0 |
| TandoorRecipes/recipes | `22b5841a` | 0 | 0 | 0 |
| paperless-ngx/paperless-ngx | `faa6f09f` | 1 | 1 | 1 |
| inventree/InvenTree | `939df605` | 0 | 0 | 0 |
| wger-project/wger | `f903c66d` | 0 | 0 | 0 |
| HumanSignal/label-studio | `2f42caec` | 1 | 1 | 1 |
| makeplane/plane | `723bb178` | 0 | 0 | 0 |
| pretix/pretix | `40ea9097` | 0 | 0 | 0 |
| netbox-community/netbox | `69978099` | 0 | 0 | 0 |
| mozilla/addons-server | `7e3374fe` | 0 | 0 | 0 |
| DefectDojo/django-DefectDojo | `cd0a4483` | 0 | 0 | 0 |

2 matches in 12 repositories, both read, both real: 100% precision, on a small sample. Tandoor's row is its run on the
final rule; pretix, netbox and addons-server also ran on the final rule. The other repositories ran before the
drf-writable-nested exemption was added. The exemption only narrows the rule, and neither of their matches is in a
writable-nested serializer.

Examples of real matches:

- label-studio `label_studio/projects/serializers.py:583`: `GetFieldsSerializer.validate_filter` returns the value only
  when it is one of `all`, `pinned_only` or `exclude_pinned`, and falls off the end otherwise. An unknown filter value
  becomes None instead of a 400, and the projects list is served as if no filter had been given.
- paperless-ngx `src/documents/serialisers.py:2407`: `PostDocumentSerializer.validate_created` returns
  `created.date()` only when the value is a datetime, and None otherwise. The field is a DateTimeField, so today only
  a null value takes that path, and None stays None. Any other value would be dropped silently: latent, but the
  pattern the rule names.

The first Tandoor run (before the drf-writable-nested exemption) reported `cookbook/serializer.py:1243`
(`StepSerializer.validate_ingredients`) and `:1417` (`RecipeSerializer.validate_steps`). Both hooks fall off the end,
but both fields are `many=True` nested serializers of a WritableNestedModelSerializer, which saves them from
`get_initial()`, so the None never reaches the database. The rule now leaves that shape alone.

A hand probe (Python `ast`, under Python 3.9, which cannot parse every file) listed every `validate`/`validate_<x>`
method with two parameters in a class with a `…Serializer` base that can fall off its end or has a bare `return`, in
the twelve repositories. Outside the two real matches above, it found only the helpers the rule leaves alone
(paperless-ngx, addons-server, DefectDojo). The probe candidates in plane (`app/serializers/page.py:205`) and
addons-server (`accounts/serializers.py:169`) return or raise on every path.

**Sources:**
- DRF API guide, [Serializers](https://www.django-rest-framework.org/api-guide/serializers/#field-level-validation):
  "Your `validate_<field_name>` methods should return the validated value or raise a `serializers.ValidationError`";
  object-level `validate()` "should return the validated values".
- DRF source, [`rest_framework/serializers.py`](https://github.com/encode/django-rest-framework/blob/master/rest_framework/serializers.py):
  `to_internal_value` sets each field to `validate_method(validated_value)`; `run_validation` asserts
  `value is not None, '.validate() should return the validated data'`.
- TestDriven.io, [Effectively Using Django REST Framework Serializers](https://testdriven.io/blog/drf-serializers/).
- Stack Overflow, [65420838](https://stackoverflow.com/questions/65420838).
- drf-writable-nested, [`mixins.py`](https://github.com/beda-software/drf-writable-nested/blob/master/drf_writable_nested/mixins.py):
  `_extract_relations` pops nested list fields from `validated_data`; `update_or_create_reverse_relations` reads
  `get_initial()`.

### `drf-practice-view-without-permission`

Rerail's own rule, not a port. DRF's `DEFAULT_PERMISSION_CLASSES` defaults to `AllowAny`, and a view's
`permission_classes` replaces the default, so in a project whose settings never set a default (or set `AllowAny`
only) a view that names no permission classes is open to anonymous users. The rule reads the project's
`REST_FRAMEWORK` settings in every non-test file of the run (a dict literal, `dict(...)`, `REST_FRAMEWORK["k"] = v`,
`REST_FRAMEWORK.update(...)`, across `settings/base.py`, `settings/production.py` and the rest) and, when they leave the
default open, reports at the name of:

- a concrete DRF view class (APIView, generic view, viewset; a subscripted base such as `GenericAPIView[Any]`
  counts) that neither assigns `permission_classes` nor defines `get_permissions()`, itself or through a base view or
  mixin of the run in any module. Concrete: no class of the run derives from it and its name does not start with
  `Base` or `Abstract`; a base view is reported through its subclasses;
- an `@api_view` function without a `@permission_classes(...)` decorator.

It leaves alone: views that set `permission_classes` to anything (`[AllowAny]` and `()` are a deliberate public
endpoint, not this rule) or define `get_permissions()`; views whose (inherited) `authentication_classes` is empty (no
request is ever authenticated: public on purpose, as addons-server's `TagListView`); a viewset whose only handlers are
`@action(permission_classes=[...])`s and whose DRF bases route nothing else (addons-server's `AddonReviewerViewSet`);
views with a base class outside the run that is not a rest_framework class, directly or through a run base
(dj-rest-auth's `LoginView`, drf-spectacular's `SpectacularAPIView`, oauth2_provider's views, Django's
`LoginRequiredMixin`: their package decides); every view of a project whose settings set a default other than
`AllowAny` in any non-test settings module (dev/prod splits count as set), or one the rule cannot read (a variable,
a concatenation, an `env.list(...)` call, a `{**BASE}` splice); runs with no `REST_FRAMEWORK` assignment; test code.

**Gaps:** a reusable app (no `REST_FRAMEWORK` in the run) is never reported, though its views inherit whatever the
host project sets; a view that checks `request.user` by hand or overrides `check_permissions()`/`initial()` is
reported; a third-party view class that itself sets no permissions hides the view; a function view wrapped by a
project decorator that calls `api_view` itself (addons-server's `bigquery_api_view`) is not seen.

**Corpus** (commits from `MANIFEST.tsv`):

| Repository | Commit | Default permission | Matches | Read | Real |
|---|---|---|--:|--:|--:|
| paperless-ngx/paperless-ngx | `faa6f09f` | unset | 2 | 2 | 2 |
| mozilla/addons-server | `7e3374fe` | unset | 7 | 7 | 7 |
| sissbruecker/linkding | `27b7303b` | IsAuthenticated | 0 | 0 | 0 |
| babybuddy/babybuddy | `1638cfb0` | BabyBuddyDjangoModelPermissions | 0 | 0 | 0 |
| TandoorRecipes/recipes | `22b5841a` | IsAuthenticated | 0 | 0 | 0 |
| HumanSignal/label-studio | `2f42caec` | set (list) | 0 | 0 | 0 |

Precision 9 of 9 read matches: each view names no permission classes in a project that sets no default, so
anonymous callers reach it. Examples: paperless-ngx `src/documents/views.py` `RemoteVersionView` and
`ReleaseNotesView`: every other API view of paperless-ngx names `IsAuthenticated` itself; these two do not, so
anonymous callers reach `/api/remote_version/` and `/api/release_notes/`, which fetch from api.github.com on their
behalf. addons-server `src/olympia/accounts/views.py` (`LoginStartView`, `AuthenticateView`),
`src/olympia/hero/views.py` (3), `src/olympia/blocklist/views.py` (`BlockViewSet`), `src/olympia/addons/views.py`
(`CompatOverrideView`): public catalog and login endpoints, open on purpose but saying so nowhere; the fix there is
the explicit `permission_classes = [AllowAny]` the message names. None of the 9 is a confirmed accidental exposure:
the rule's value is making public endpoints explicit, so that every new view in such a project gets a decision.
A hand listing of addons-server (Python `ast`) also has `TagListView`, `ReplacementAddonView`,
`AddonBrowserMappingView` (`authentication_classes = []`: public by design) and `AddonReviewerViewSet` (every action
names `AllowAnyKindOfReviewer`); the rule leaves both shapes alone, and both are cases.

**Sources:**
- DRF API guide, [Permissions](https://www.django-rest-framework.org/api-guide/permissions/): "If not specified,
  this setting defaults to allowing unrestricted access: `'rest_framework.permissions.AllowAny'`".
- DRF API guide, [Settings](https://www.django-rest-framework.org/api-guide/settings/): `DEFAULT_PERMISSION_CLASSES`
  defaults to `AllowAny`.
- OWASP [Django REST Framework Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Django_REST_Framework_Cheat_Sheet.html):
  "unless the default permission class is changed, everybody can access every view by default"; change the default.
- OWASP API Security Top 10 2023, [API5 Broken Function Level Authorization](https://owasp.org/API-Security/editions/2023/en/0xa5-broken-function-level-authorization/).
