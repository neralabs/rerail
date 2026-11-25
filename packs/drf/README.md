# Rerail rule pack: Django REST framework

Rerail's own Django REST framework rules, from the DRF docs and maintainers' review advice, for safe serializers, permissions, querysets and pagination.

This pack is private while its rules are written and reviewed: it is not published to npm, and is installed from
a checkout with `rerail packs add packs/drf`.

## Rules

In reading order, by the harm each prevents. Precision is the share of read corpus matches a reviewer would ask to
fix, from the re-measured runs in each rule's details.

| Rule | Mode | On | Catches | Precision (read) |
|---|---|---|---|--:|
| `drf-practice-object-permission-skipped` | Steer | yes | DRF practice: a handler that reads or changes its row itself, skipping the view's object permission | 3 of 3 |
| `drf-practice-view-without-permission` | Steer | yes | DRF practice: a view that sets no permission classes in a project whose default permission is AllowAny | 9 of 9 |
| `drf-practice-serializer-exposes-secret-field` | Log | no | DRF practice: a ModelSerializer that returns a password, token or API key | 7 of 15 |
| `drf-practice-declared-field-meta-ignored` | Steer | yes | DRF practice: read_only_fields or extra_kwargs naming a declared serializer field | 8 of 8 |
| `drf-practice-validate-returns-value` | Steer | yes | DRF practice: serializer validate_<field>() or validate() that can end without returning | 5 of 5 |
| `drf-practice-page-size-uncapped` | Steer | yes | DRF practice: pagination that lets the client choose the page size with no upper bound | 6 of 6 |
| `drf-practice-related-field-not-prefetched` | Log | yes | DRF practice: a list view's serializer reads a relation its queryset does not load | 31 of 31 |
| `drf-practice-list-without-pagination` | Log | no | DRF practice: list endpoint with no pagination, or a pagination class with no page size | 7 of 31 |

## Rule details

### `drf-practice-object-permission-skipped`

Rerail's own rule, not a port. Django REST framework runs a permission class's `has_object_permission` only inside
`get_object()` and `check_object_permissions(request, obj)`. A handler that fetches the row it acts on itself
(`get_object_or_404(Book, pk=pk)`, `Book.objects.get(pk=pk)`) and calls neither lets any user who passes the
view-level `has_permission` (usually: any signed-in user) read, change or delete any row by its id: OWASP API1,
broken object level authorization.

It reports, in a handler whose permission classes check objects, a single-row fetch and a write without a fetch:

- **The handler's permission classes check objects.** A permission class checks objects when its
  `has_object_permission` (its own or a base's) reads its `obj` argument, or it is (derives from)
  `DjangoObjectPermissions`. The classes are the view's `permission_classes`, its own or a base view's in another
  module (also `api_settings.DEFAULT_PERMISSION_CLASSES + [...]`), a `get_permissions()` with one return of a list of
  instances (`return [IsOwner()]`), else the settings' `DEFAULT_PERMISSION_CLASSES`, which may name the class by a
  dotted string. `@action(permission_classes=[...])` replaces them for its handler. An `@api_view` function uses its
  `@permission_classes([...])`, else the settings default. Classes resolve through lib/drf's `drf_cls`: across the
  project's import root and package re-exports.
- **The handler acts on one row:** `get`, `put`, `patch`, `delete`, `retrieve`, `update`, `partial_update`,
  `destroy`, an `@action(detail=True)`, or an `@api_view` function.
- **A fetch reads one of the view's rows:** `get_object_or_404(M, …)`, `get_object_or_404(<queryset>, …)`,
  `<queryset>.get(…)`, or a filtered chain ended by `.get()`, `.first()`, `.last()`, `.latest(…)`, `.earliest(…)` or
  `[0]`. Or **a write skips the fetch:** `<queryset>.filter(<URL kwarg>).delete()` or `.update(…)`. Querysets are the
  view's model's (`M.objects`, `_default_manager`, a manager the model declares, `QS.as_manager()`), `self.queryset`,
  `self.get_queryset()`, `self.filter_queryset(…)`, any chain from one, and a local name bound to one. In a view
  without a model and in a function view: any model's, when the lookup reads a URL kwarg (a handler parameter other
  than `self`, `request` and `format`, `self.kwargs`, `kwargs[...]`, or a local bound to one).
- **Nothing checks the row:** neither the handler nor a method of the view it calls through `self.<name>(…)` calls
  `get_object()`, `check_object_permissions()`, `has_object_permission()`, a DRF detail handler of the view
  (`super().update()`, `self.retrieve()` of a mixin, which call `get_object()`), or a permission function given the
  requester (`project.has_permission(request.user)`, `request.user.has_perm("change_book", book)`,
  `has_perms_owner_aware(user, …, doc)` with `user = request.user`), and the handler does not refuse requests itself
  (raises `PermissionDenied`, answers 403 or `HttpResponseForbidden`).

The messages lead with `self.check_object_permissions(request, obj)`, which never changes behaviour for an allowed
user (`get_object()` also applies `filter_queryset()` and `lookup_field`, so swapping a lookup for it can change the row
found). A function view gets its own message: DRF never runs `has_object_permission` for one, so the fix is comparing
the row's owner to `request.user`, or filtering by it.

```python
class NoteDetail(APIView):
    permission_classes = [IsOwner]
    def delete(self, request, pk):
        Note.objects.filter(pk=pk).delete()                 # reported: deletes any user's note

class NoteReader(APIView):
    permission_classes = [IsOwnerOrReadOnly]                # True for SAFE_METHODS before reading obj
    def get(self, request, pk):
        note = get_object_or_404(Note, pk=pk)               # not reported: the skipped check allows every GET
```

It leaves alone: lookups that pass `request.user`, an attribute of it (`organization=request.user.active_organization`)
or a name bound to one (in a write: in the filter chain, not in the new values); views whose `get_queryset()` reads
`request.user`, or a method of which (NetBox's `initial()`) sets `self.queryset` from an expression reading it; list
and create handlers; a handler that serves only safe methods (`get`, `head`, `options`, `retrieve`, a GET-only
`@action` or `@api_view`) when every object permission on it answers `True` for safe methods first: its
`has_object_permission` (own or inherited) starts with `if request.method in SAFE_METHODS: return True` or
`return request.method in SAFE_METHODS or …`, `SAFE_METHODS` by any alias, a literal list or a constant. That is read
from the body, never from the class name, and `DjangoObjectPermissions` counts as read-open unless a subclass sets
`perms_map`. Also left alone: views whose permission classes check no objects, or whose `has_object_permission`
never reads the object (Tandoor's `CustomIsGuest`); plain Django views; test files.

**Gaps:** a fetch of another model than the view's in a view with a model (a project row fetched to authorise a task
row) is not reported; a lookup in a helper the handler calls is not reported (only the handler's own body is read);
a check in a module-level helper is seen only when its name says permission (`perm`, `access`, `allow`, `authori`)
and it is given the requester; a `get_permissions()` with several returns (branching on `self.action`) and a
`permission_classes` property are not read; reads that stop at the queryset (`.values()`, `.exists()`) are not
reported; managers inherited from abstract bases are not seen; a lookup scoped by tenant (`space=request.space`)
counts as unscoped.

**Sources:** DRF API guide, [Permissions](https://www.django-rest-framework.org/api-guide/permissions/) ("Object
level permissions are run by REST framework's generic views when `.get_object()` is called"; "the view code should
explicitly call `.check_object_permissions(request, obj)` … Function-based views will need to check object
permissions explicitly"); TestDriven.io, [Permissions in Django REST Framework](https://testdriven.io/blog/drf-permissions/)
(the `get_object_or_404` + `check_object_permissions` example); OWASP API Security Top 10 2023,
[API1 Broken Object Level Authorization](https://owasp.org/API-Security/editions/2023/en/0xa1-broken-object-level-authorization/);
Stack Overflow, [has_object_permission not called](https://stackoverflow.com/questions/54783424/has-object-permission-not-called).
No linter models DRF permissions; the django pack has no permission rule.

**Corpus** (`~/rerail-bench/python-arch/corpus`, commits from `MANIFEST.tsv`; the review's runs of 2026-11-25, and runs
`drf-fix-ops-<repo>` of the reworked rule on Tandoor, label-studio and paperless-ngx, which gave the same numbers):

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

The write shape is rare in mature code: a search of label-studio, paperless-ngx, netbox, wger and babybuddy for
`.filter(pk|id=…).delete()/.update()` found one hit, a netbox signal helper, not a handler. The reviewer's
counterexample repository gives 5 findings on the reworked rule: the `IsOwnerOrReadOnly` GET is silent, and the
`filter().delete()`, `filter().update()`, `filter()[0]` and `get_permissions()` shapes are reported.

Mode: `steer`, on (3 of 3 read matches real). The sample is small: the corpus apps mostly use `get_object()`, which is
the point of the rule; it is meant to catch new handlers that do not.

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

**Corpus** (commits from `MANIFEST.tsv`; runs `drf-fix-vwp-paperless` and `drf-fix-vwp-addons` of the final rule gave
the same 2 and 7 matches):

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
the explicit `permission_classes = [AllowAny]` the message names. paperless-ngx's two look unintended; the 7 of
addons-server are public by design: the rule's value there is making public endpoints explicit, so that every new view
in such a project gets a decision. A hand listing of addons-server (Python `ast`) also has `TagListView`,
`ReplacementAddonView`, `AddonBrowserMappingView` (`authentication_classes = []`: public by design) and
`AddonReviewerViewSet` (every action names `AllowAnyKindOfReviewer`); the rule leaves both shapes alone, and both are
cases.

Mode: `steer`, on.

**Sources:**
- DRF API guide, [Permissions](https://www.django-rest-framework.org/api-guide/permissions/): "If not specified,
  this setting defaults to allowing unrestricted access: `'rest_framework.permissions.AllowAny'`".
- DRF API guide, [Settings](https://www.django-rest-framework.org/api-guide/settings/): `DEFAULT_PERMISSION_CLASSES`
  defaults to `AllowAny`.
- OWASP [Django REST Framework Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Django_REST_Framework_Cheat_Sheet.html):
  "unless the default permission class is changed, everybody can access every view by default"; change the default.
- OWASP API Security Top 10 2023, [API5 Broken Function Level Authorization](https://owasp.org/API-Security/editions/2023/en/0xa5-broken-function-level-authorization/).

### `drf-practice-serializer-exposes-secret-field`

Rerail's own rule, not a port. A ModelSerializer returns every model field its `Meta` lets in: all of them with
`fields = "__all__"`, every one an `exclude` does not name, or the ones `fields` lists. When one of them holds a
secret, the serializer's output carries it (OWASP API3, Broken Object Property Level Authorization). The rule resolves
the serializer's `Meta.model` to a model of the run in any module, reads its fields, its own and those it inherits from
the run's model bases (an abstract base in a third module), and reports:

- at the `Meta` value that lets the secret in: the `"__all__"` string or the `exclude` value (once per secret field),
  or the listed string in `fields` (lists joined with `+`, `Base.Meta.fields`, and a module constant such as
  `fields = BUCKET_FIELDS`, reported at the `fields` value when the list is outside the class);
- at a `SerializerMethodField` in the output whose method (`get_<name>`, or `method_name=`) returns
  `obj.<secret field>`.

A field is a secret when its whole name says so and its class can hold one. Detection is by name, the one thing a
secret column has in common, which is why the rule is in `log`. Names: `[*_]password[s][_hash]` (`passwd`,
`passphrase`), `[*_]secret[s][_key|_access_key|_token|_value]`, `[*_]token[_key|_secret|_value]`,
`[*_]tokens_(key|secret|value)`, `[*_](api|secret|private|signing|encryption)[_]key[s]`,
`[*_](otp|totp|hotp)[_secret|_seed|_key]`, `[*_](mfa|2fa)_(secret|seed|key)`, `[*_]credential[s]`. The plural
`tokens` alone (NLP tokens, stored LLM token data) and `access_key` without `secret` (an object-storage key id, whose
secret is `secret_access_key`) name non-secrets more often than secrets and are not on the list. Booleans, numbers,
dates and times, durations, auto keys and relations are left out, so `password_changed_at`, `token_expires`,
`tokens_used`, `has_password` and a BooleanField `secret_santa` never match. A model deriving from Django's
`AbstractBaseUser`, `AbstractUser` or `User` (and `get_user_model()`, or a module's `User = get_user_model()`) has
Django's `password` hash. A model field class the project defines (`EncryptedTextField`) or another package's
(`AESField`) counts when its name ends in `Field`. Models, model bases and serializer bases resolve through lib/drf's
`drf_cls`: across a project's import root (label-studio imports `label_studio/organizations/models.py` as
`organizations.models`) and through package re-exports.

It leaves alone a secret the serializer hides: a declaration of the name (in the serializer or a serializer base it
inherits it from) as `x = None`, a field with `write_only=True`, a `HiddenField` or `SerializerMethodField`, a field
with another `source=`, or a field class that is not rest_framework's own (paperless-ngx's `ObfuscatedPasswordField`);
with no declaration, `Meta.extra_kwargs` giving it `write_only: True` (a dict literal, `dict(...)` or a module
constant); and a serializer with a `to_representation` (its own or a base's, a plain mixin included) whose class, or a
base class, names the field as a string outside its `Meta` (`data.pop("password")`, or label-studio's
`secure_fields = [...]` list that `to_representation` pops in a loop). Plain `Serializer`s, serializers whose model is
outside the run (other than Django's users), and test code are left alone.

**Gaps:** tokens meant to be shown are reported, since no name tells them from credentials: a token returned once by
the endpoint that creates it (plane's `APITokenSerializer`, pretix's device tokens), invite and share-link tokens, an
export job's id named `token`; a serializer only ever used to validate input, never rendered (pretix's
`OrderPositionChangeSerializer`), is reported; a declared plain field next to `extra_kwargs` write_only is reported
(DRF ignores `extra_kwargs` for declared fields, so it is exposed); a secret under a name the list misses (`key`,
`pin`, `salt`) is not; a plain `Serializer` declaring `secret_access_key = CharField()` is not; `fields` built by a
call or a comprehension (plane's `[f.name for f in User._meta.fields if f.name != "password"]`) is not read; a
`to_representation` that drops the field through a helper in another module is reported.

**Corpus** (commits from `MANIFEST.tsv`; runs `drf-fix-sx-<repo>` of the reworked rule on label-studio, plane,
paperless-ngx, netbox, DefectDojo, Tandoor and InvenTree gave the review's matches; pretix's row is the review's run,
its rerun was stopped with the machine's other engine runs; judged as the review judged them):

| Repository | Commit | Matches | Read | Real | Shown by design | Not a secret |
|---|---|--:|--:|--:|--:|--:|
| HumanSignal/label-studio | `2f42caec` | 4 | 4 | 3 | 1 | 0 |
| makeplane/plane | `723bb178` | 4 | 4 | 0 | 3 | 1 |
| paperless-ngx/paperless-ngx | `faa6f09f` | 1 | 1 | 1 | 0 | 0 |
| netbox-community/netbox | `69978099` | 1 | 1 | 1 | 0 | 0 |
| DefectDojo/django-DefectDojo | `cd0a4483` | 2 | 2 | 2 | 0 | 0 |
| pretix/pretix | `40ea9097` | 3 | 3 | 0 | 2 | 1 |
| the other 7 DRF repositories of the review | | 0 | 0 | 0 | 0 | 0 |

Precision 7 of 15 read matches (47%), so the rule is in `log` and off. Real: label-studio
`label_studio/projects/serializers.py:342` (`ProjectSerializer` lists `task_data_password`, the task storage's
credentials, returned to every project reader) and `webhooks/serializers_for_hooks.py:30` (`ProjectWebhookSerializer`,
`"__all__"`: `task_data_password` and `token`, posted to the webhook's URL); paperless-ngx
`src/documents/serialisers.py:3269` (`WorkflowAction.passwords`, the PDF passwords a workflow tries); netbox
`netbox/extras/api/serializers_/events.py:67` (`Webhook.secret`, the HMAC key); DefectDojo
`dojo/system_settings/api/serializer.py:9` (`"__all__"`: `jira_webhook_secret` and `slack_token`, to superusers).
Shown by design: label-studio's organization invite token, plane's API token (returned once by the create view) and
workspace and project invite tokens, pretix's device `api_token` and `initialization_token`. Not a secret: plane's
`ExporterHistory.token` (an export job's id) and pretix's input-only `OrderPositionChangeSerializer`. No corpus match
depended on the dropped names (`tokens`, `access_key`), and no corpus serializer uses the new method-field or constant
shapes. Of the reviewer's counterexamples, `tokens` and `access_key` are silent; `secret_access_key` under `"__all__"`,
the method field and the constant are reported; the share-link token is still reported (a documented gap).

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

### `drf-practice-declared-field-meta-ignored`

Rerail's own rule, not a port. ModelSerializer applies `Meta.read_only_fields` and `Meta.extra_kwargs` only to the
fields it builds from the model. A field the serializer declares itself (`username = serializers.CharField()`), in its
own body or in a base serializer, ignores both options, and DRF says nothing. A field listed read-only stays writable,
so any client can set it (OWASP API3, mass assignment). `extra_kwargs = {"password": {"write_only": True}}` on a
declared `password` leaves the password in every response.

The rule matches a serializer outside test code (ModelSerializer, HyperlinkedModelSerializer, or a plain Serializer,
which ignores its whole Meta) when all of these hold:

- it declares field `x` as a field call: a rest_framework field or a subclass, a nested serializer of the run, or a
  `…Field`/`…Serializer` class from another package. The declaration is its own, or the nearest base serializer's in
  any module of the run, when no class in between reassigns `x` (`x = None` removes the field). Classes resolve
  through lib/drf's `drf_cls`: across the project's import root and package re-exports.
- its Meta names `x`. The Meta can be its own, inherited through `class Meta(Base.Meta)`, or its base's when it has
  no Meta of its own. The name counts in `read_only_fields` (a list or tuple, `+`, `*splat`, a module constant here
  or imported, or `Other.Meta.read_only_fields`) or as a key of `extra_kwargs` (a dict literal, `dict(...)` or a
  module constant).

Each serializer and field is reported once:

- at the name in the Meta option when the name is written in the serializer's own Meta;
- otherwise at the field's declaration when the serializer declares the field;
- otherwise (both inherited) at the class name, unless a base serializer already has the same name and declaration.
  That base is reported, and its subclasses are not.

The fix starts at the reported place. For `read_only_fields` there are two messages. When a serializer that carries
the entry (the serializer, a run base whose Meta lists the field, or a run subclass with the same entry) writes the
field on purpose, the message says to drop the entry: a `validate_<x>` method, a `create`, `update`, `validate`, `save`
or `to_internal_value` that reads `x` from its input (`validated_data["x"]`, `.get`, `.pop`, `self.validated_data`),
or a declared field class of the run that parses input (its own `to_internal_value`). Otherwise the message offers
both: `read_only=True` on the declared field if clients must not write it, else drop the entry; it never tells an
agent to set `read_only=True` unconditionally, which would break a serializer that writes the field (linkding's
`tag_names`). A plain Serializer gets its own message. The message for `extra_kwargs` says to pass those arguments to
the declared field.

It leaves alone:

- `read_only_fields` entries for fields that are read-only anyway: `read_only=True` written out or through `**RO` (a
  module-level dict, here or imported), a SerializerMethodField, ReadOnlyField, HiddenField, HyperlinkedIdentityField,
  StringRelatedField or a subclass of one; an `__init__` or `get_fields` of the serializer or a run base that sets
  `self.fields["x"].read_only = True` (directly or in a loop over literal names): the entry is dead but harmless;
- `extra_kwargs` entries whose every option the declared call already passes with the same text. `read_only: True` on
  a read-only field also counts as passed, and so does `required: False` on a field with a `default=` (label-studio's
  `created_by`). An empty entry is left alone too;
- a serializer whose own or a run base's `__init__` or `get_fields` reads the option by name. Such a mixin applies the
  option to declared fields itself (addons-server's version serializer sets `fields[name].read_only` from
  `Meta.read_only_fields`);
- for `read_only_fields`, a serializer that refuses all input: its own or a run base's `validate`,
  `to_internal_value` or `run_validation` raises unconditionally, as pretix's `OrderPositionSerializer` does;
- a run class that is not a DRF field (InvenTree's `OptionalField` placeholder), names a plain Serializer's
  `read_only_fields` lists that it does not declare, fields named in a ModelSerializer's Meta but not declared (DRF
  honours both options for those), and test code.

A field named in both options is reported for `read_only_fields` only.

**Gaps:**

- With a diamond of base serializers, the rule takes the nearest declaration along each path instead of DRF's MRO.
- A project field class whose own `__init__` forces `read_only` is reported.
- `extra_kwargs` built by a call other than `dict()` or by a comprehension is not read.
- Writes made outside the serializer (a view's `perform_update` reading `serializer.validated_data["x"]`) or through a
  helper are not seen, so such a field gets the two-option message; an `__init__` loop over a constant is not read.

**Corpus** (commits from `MANIFEST.tsv`; the review's runs of 2026-11-25 over 13 repositories, and runs
`drf-fix-dfm-<repo>` of the reworked rule on InvenTree, Tandoor, linkding, paperless-ngx and netbox, which gave the
same 8 matches):

| Repository | Commit | Matches | Read | Real | Message |
|---|---|--:|--:|--:|---|
| inventree/InvenTree | `939df605` | 6 | 6 | 6 | 5 two-option, 1 written on purpose |
| TandoorRecipes/recipes | `22b5841a` | 1 | 1 | 1 | two-option |
| sissbruecker/linkding | `27b7303b` | 1 | 1 | 1 | written on purpose |
| the other 10 DRF repositories of the review | | 0 | 0 | 0 | |

Precision 8 of 8 read matches (100%): each is a dead Meta entry a reviewer would ask to fix, and 5 are bugs today.
The two writes-intended matches the review named are handled: linkding's `tag_names` (popped in `create()` and
`update()`) and InvenTree's `email` (validated by `ExtendedUserSerializer.validate_email`) get "drop the entry";
InvenTree's `username` has no write evidence in the run and gets the two-option message. Of the reviewer's
counterexamples, `SlugField(**RO)` and the `__init__` that sets `read_only` are silent; the plain Serializer, the
`READONLY` constant, the `extra_kwargs` `max_length` and the `CurrentUserDefault` author are reported.

Examples of real matches:

- InvenTree `src/backend/InvenTree/users/serializers.py:607`: `UserCreateSerializer` lists `is_staff` and
  `is_superuser` in its read_only_fields to "prevent creation of users with superuser or staff permissions". Both are
  writable BooleanFields declared on `ExtendedUserSerializer`, so `is_staff` is writable on create; `is_superuser` is
  still guarded by `validate_is_superuser`. Two matches.
- InvenTree `users/serializers.py:214`: `UserSerializer` lists `username` and `email` read-only and declares both as
  writable fields; the staff user-edit view's `ExtendedUserSerializer` inherits the entry and edits both, so the fix
  there is dropping the entry.
- InvenTree `part/serializers.py:577`: `creation_user` is a writable `PrimaryKeyRelatedField`, listed read-only;
  `create()` sets it from `request.user`, but an update accepts any user. InvenTree `common/serializers.py:860`
  declares `filename` with `source='basename'`, a read-only property, so an update that sends it fails on save.
- Tandoor `cookbook/serializer.py:843`: `RecipeImportSerializer` (WritableNestedModelSerializer) lists `storage`
  read-only, and `storage` is a writable nested `StorageSerializer`, so a client can create or change storage backends
  through a recipe import.
- linkding `bookmarks/api/serializers.py:90`: `tag_names` is listed read-only, but `create()`/`update()` read it from
  `validated_data`. The entry is dead and misleading; the message says to drop it.

Mode: `steer`, on.

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

It reports the method's name when a path can reach the end of the body, each bare `return`, and each `return None` in
`validate()` (DRF's `assert value is not None` rejects it every time). The flow analysis is the one of the django
pack's `django-practice-clean-field-returns-value`: `if`/`elif`/`else`, `try`/`except`/`else`/`finally`, `with`,
loops with `else` and `break`, `while True`, and `match` with or without a catch-all case. A statement that only calls
something that always raises ends its path: `self.fail("key")` (DRF's `Field.fail` raises `ValidationError`, unless
the class or a run base defines its own `fail`), and a call of a function or method of the run whose every path raises
(`reject("bad")` imported from another module, `self._reject()`), resolved through imports and the class's methods.

It leaves alone:

- a method whose every path returns or raises, a body that only raises (`raise NotImplementedError`), a path that
  ends in `self.fail(...)` or in a call of a run function that always raises;
- `return None` written out in a `validate_<x>`, as a value chosen on purpose;
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
- A helper that raises only through another helper, or only on some paths, does not end the caller's path.
- A hook on a mixin class that does not itself derive from a serializer is not checked.

**Overlap:** none in the DRF ecosystem. Ruff's RET503 (implicit return) is not among the python pack's ports
(`packs/python/pack.json` ports RET501, RET502 and RET505-RET508). RET503 would flag every function that returns a
value on some path, without knowing which methods DRF reads the value of. RET502 (in the python pack) flags a bare
`return` only in a function that also returns a value elsewhere, and says nothing about a hook that falls off the end.

**Corpus** (commits from `MANIFEST.tsv`; the review's runs of 2026-11-25 over 13 repositories, and runs
`drf-fix-vrv-<repo>` of the final rule on escriptorium, label-studio, paperless-ngx, Tandoor and InvenTree, which gave
the same matches):

| Repository | Commit | Matches | Read | Real |
|---|---|--:|--:|--:|
| UB-Mannheim/escriptorium | `7e06bd31` | 3 | 3 | 3 |
| HumanSignal/label-studio | `2f42caec` | 1 | 1 | 1 |
| paperless-ngx/paperless-ngx | `faa6f09f` | 1 | 1 | 1 |
| the other 10 DRF repositories of the review | | 0 | 0 | 0 |

Precision 5 of 5 read matches (100%); 1 is a bug today, 4 are latent. The fix (`return value`) cannot break working
code: the field keeps the value it arrived with. No corpus serializer has `return None` in `validate()` or ends a
hook in `self.fail(...)`; the reviewer's counterexamples (`reject("bad")`, `self.fail("invalid")`) are now silent.

Examples of real matches:

- label-studio `label_studio/projects/serializers.py:583`: `GetFieldsSerializer.validate_filter` returns the value only
  when it is one of `all`, `pinned_only` or `exclude_pinned`, and falls off the end otherwise. An unknown filter value
  becomes None instead of a 400, and the projects list is served as if no filter had been given.
- paperless-ngx `src/documents/serialisers.py:2407`: `PostDocumentSerializer.validate_created` returns
  `created.date()` only when the value is a datetime, and None otherwise. The field is a DateTimeField, so today only
  a null value takes that path, and None stays None. Any other value would be dropped silently: latent, but the
  pattern the rule names.
- escriptorium `app/apps/api/serializers.py:1181`, `:1189`, `:1199`: `validate_iiif_uri`, `validate_mets_uri` and
  `validate_upload_file` never return; they set `self.file`, and the three fields become None in `validated_data`,
  which `validate()` and `process()` only test with `'x' in data`: latent.

Mode: `steer`, on.

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

### `drf-practice-page-size-uncapped`

Rerail's own rule, not a port. DRF's paginators take the page size from the query string when the class names a
parameter for it, and cap it only by `max_limit` (`LimitOffsetPagination`) or `max_page_size`
(`PageNumberPagination`, `CursorPagination`). Both default to `None`, so `?limit=100000000` or
`?page_size=100000000` makes one request load and serialise every row of the table. The rule reads the project's
`REST_FRAMEWORK` settings, its views and its pagination classes across files and reports, outside test code:

- the built-in `rest_framework.pagination.LimitOffsetPagination` named as `DEFAULT_PAGINATION_CLASS` (a dotted
  string or a class reference, reported at the value), as a pagination class: a class body's
  `pagination_class = ...`, `self.pagination_class = ...`, `@action(pagination_class=...)` (reported at the value), or
  instantiated in code (`paginator = LimitOffsetPagination()` in an `@api_view`, reported at the call);
- a project class deriving from `LimitOffsetPagination`, directly or through the run's classes in any module, with
  no `max_limit` of its own or of a run base (`max_limit = None` counts as none) and no capping `get_limit()`
  (reported at the class name);
- a project class deriving from `PageNumberPagination` or `CursorPagination` whose own or inherited
  `page_size_query_param` is a string, with no `max_page_size` and no capping `get_page_size()` (reported at the
  class name).

It leaves alone: a `max_limit`/`max_page_size` set to any value other than `None`, however large, assigned in
`__init__` (`self.max_limit = ...`) or defined as a property; a `get_limit()`/`get_page_size()` override, its own or a
run base's (netbox's `NetBoxPagination` caps in `get_limit()`), unless it hands the client's value back unchecked: a
`return super().get_page_size(request)` in a method that neither raises nor calls `min()`; a `paginate_queryset()`
override, its own or a run base's, that calls `min()` or raises (it clamps the size itself:
`self.limit = min(self.get_limit(request) or 50, 500)`);
`limit_query_param = None` and `page_size_query_param = None`; `PageNumberPagination` and `CursorPagination` without
`page_size_query_param` (the client cannot change the size), built-in or subclassed; a setting or `pagination_class`
that names a project class (the class is judged where it is written); a project class used only as a base (a run class
derives from it, and no setting, `pagination_class` or call names it), whose subclasses are judged instead; test code.

**Gaps:** a cap read from configuration (`max_page_size = settings.TASK_API_PAGE_SIZE_MAX`) is trusted even when the
configured value is `None` or huge; a cap far above any real page is left alone (paperless-ngx's `StandardPagination`
has `max_page_size = 100000`); `page_size_query_param` set in `__init__` is not seen; an override that clamps through a
helper but also returns `super().get_page_size(...)` unchanged on some path is reported, and so is a
`paginate_queryset()` that clamps through a helper without `min()`; pagination classes of third-party packages are
not followed.

**Corpus** (commits from `MANIFEST.tsv`; runs `drf-fix-psu-<repo>` of the final rule on linkding, babybuddy,
label-studio, netbox, InvenTree, Tandoor and paperless-ngx gave the same matches; DefectDojo's row is the review's run,
its rerun was stopped with the machine's other engine runs):

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
`StripCountAnnotationsPaginator` (capped in `get_limit()`). Of the reviewer's counterexamples, the class that clamps in
`paginate_queryset()` is silent and the built-in class instantiated in an `@api_view` is reported; `max_limit = 10 ** 9`
stays silent by design.

Mode: `steer`, on.

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

Rerail's own rule, not a port. A list view serialises every row of its queryset; a serializer field that reads a
related object (or collection) of the row runs one query per row unless the queryset loaded it. The view, the
serializer and the model usually sit in three modules, which no single-file check joins.

It matches a DRF list view (`ListAPIView`, `ListCreateAPIView`, `ModelViewSet`, `ReadOnlyModelViewSet`,
`ListModelMixin`, or a viewset's own `list`) outside test code whose (inherited) `serializer_class` is a ModelSerializer
of the run with model `mc`, when a field of that serializer reads relation `r` of `mc` for each row:

- a declared nested serializer on `r` (by its name or `source="r"`), or a declared field with a dotted `source="r.x"`;
- a declared field that renders the related object (`StringRelatedField`, `SlugRelatedField`, `CharField(source="r")`):
  every rest_framework field but `PrimaryKeyRelatedField` and `HyperlinkedRelatedField`, which read the key;
- any field on a many relation (a ManyToManyField, a reverse ForeignKey), `PrimaryKeyRelatedField(many=True)` included;
- a ManyToManyField or reverse relation named in `Meta.fields`, or any forward relation with `Meta.depth >= 1`.

The queryset must be one the rule can read: the view's (inherited) `queryset`, and every `get_queryset()` it runs,
return chains from `mc.objects`, `self.model.objects` (the view's `model = mc`), `self.queryset`,
`super().get_queryset()` or `self.filter_queryset(...)` of one; and no `select_related(...)`/`prefetch_related(...)`
of the view or its run bases loads `r` (a lookup `r` or `r__…`, `Prefetch("r")`, a string of a module-level list
splatted in, or `select_related()` for a forward one-row relation).

It reports once per view and serializer, listing every relation the view misses as the call that loads them, where
the fix goes: the view's own `get_queryset()`, else its `queryset =`, else its `serializer_class =`, else the class
name (unless a run base view listing with the same serializer is reported instead).

```python
class BookSerializer(serializers.ModelSerializer):
    author = AuthorSerializer(read_only=True)
    class Meta:
        model = Book
        fields = ["id", "author", "tags"]        # tags: ManyToManyField

class BookViewSet(viewsets.ModelViewSet):
    queryset = Book.objects.all()                # reported once: select_related("author") and prefetch_related("tags")
    serializer_class = BookSerializer
```

It leaves alone:

- a plain ForeignKey in `Meta.fields` (`PrimaryKeyRelatedField` reads `author_id`), write-only fields;
- a list that is not the serializer's: a `list()` of the view or a run base that never calls `self.get_serializer(...)`
  or `super().list(...)` (`return Response(BookLiteSerializer(qs, many=True).data)`), and a `get_serializer_class()` or
  `get_serializer()` of the view or a run base;
- relations loaded elsewhere: `prefetch_related_objects(...)` (Django's, by import) in the view's or a run base's
  `list()`, `paginate_queryset()`, `get_queryset()` or `filter_queryset()`; a `select_related`/`prefetch_related`
  argument the rule cannot read (a variable, `*self.related`);
- a list of one row: the queryset filters a field of `mc` that is a OneToOneField (any `…OneToOneField`), `unique=True`
  or `primary_key=True`, or `pk`/`id` (`UserPreference.objects.filter(user=self.request.user)`);
- querysets the rule cannot see (a helper, a custom manager or QuerySet method, a related manager, a manager of the run
  with its own `get_queryset`, another root model), serializers with their own or a run base's `to_representation`,
  detail-only views and test code.

**Gaps:** `SerializerMethodField` bodies are not read, so `get_author_name(self, obj): return obj.author.name` is a
miss, and no rule of this pack covers it; a `list()` serialising `BookSerializer(self.get_queryset(), many=True)`
directly is left alone; one unique-field filter on any path counts as one row; `prefetch_related_objects` counts as
loading every relation; only a source's first step is checked (`source="author.country.name"` is satisfied by
loading `author`); a `Prefetch("r", to_attr="x")` counts as loading `r`; nested serializers' own relations are not
followed; a string relation target resolves by class name. plane gives no match: most of its viewsets import their
base through `from .. import BaseViewSet` in directories without `__init__.py`, which the core name resolution does
not place, and the rest serialise with request-dependent serializers (`DynamicBaseSerializer`, own
`to_representation`), which the rule leaves alone by design.

**Corpus** (runs `drf-fix-rnp-<repo>` of the reworked rule, and `drf-final-<repo>` of the whole pack on the Junction
fleet for the repositories under 1,000 files; every match read against its view, serializer and queryset; "before" is
the review's per-field count):

| Repository | Matches (before) | Real |
|---|--:|--:|
| TandoorRecipes/recipes | 15 (45) | 15 |
| paperless-ngx/paperless-ngx | 2 (21) | 2 |
| inventree/InvenTree | 4 (7) | 4 |
| DefectDojo/django-DefectDojo | 2 (5) | 2 |
| wger-project/wger | 2 (2) | 2 |
| UB-Mannheim/escriptorium | 6 (0) | 6 |
| makeplane/plane | 0 (0) | 0 |
| HumanSignal/label-studio | 0 (0) | 0 |

Precision 31 of 31 read matches (100%), one finding per view. escriptorium's 6 are new: its views import their models
and serializers through its import root (`app/apps/core/models.py` as `core.models`), which `drf_cls` now resolves; each is a real
per-row query (`DocumentMetadataViewSet` and `PartMetadataViewSet` nest `key`, `ImageAnnotationViewSet` and
`TextAnnotationViewSet` nest `components` many, `TextualWitnessViewSet` and `VirtualCollectionViewSet` read
`owner.username`). The review's 7 false positives (Tandoor's
`UserPreferenceViewSet` and `SearchPreferenceViewSet`, one row per user through an `AutoOneToOneField`) are gone.
Examples: Tandoor `cookbook/views/api.py:705` `SpaceViewSet` (12 forward relations and an M2M), `:1690` `StepViewSet`;
paperless-ngx `src/documents/views.py:5236` `WorkflowActionViewSet` prefetches the `assign_*` M2Ms but not the
`remove_*` ones, `email` or `webhook`; DefectDojo `dojo/notes/api/views.py:30` (`author`, `editor`, `note_type`,
`history`). Of the reviewer's counterexamples only the plain `Book.objects.all()` is reported; the splat, lighter
`list()` and `prefetch_related_objects` shapes are silent.

Mode: `log`, on. Precision is high, but a steer would pull agents into querysets on unrelated edits.

**Sources:** DRF API guide, [Generic views](https://www.django-rest-framework.org/api-guide/generic-views/)
(`get_queryset`, "Avoiding N+1 Queries"); Haki Benita,
[Improve Serialization Performance in Django Rest Framework](https://hakibenita.com/django-rest-framework-slow);
Django, [`select_related` and `prefetch_related`](https://docs.djangoproject.com/en/stable/ref/models/querysets/#prefetch-related).
No static linter joins view, serializer and model; nplusone and django-zen-queries catch it at run time in tests. The
django pack's `select-related-in-loop` covers explicit loops only.

### `drf-practice-list-without-pagination`

Rerail's own rule, not a port. DRF paginates nothing by default: `DEFAULT_PAGINATION_CLASS` and `PAGE_SIZE` are both
`None`, so a list endpoint with no pagination class serialises every row its queryset holds on every request. The rule
reads the project's `REST_FRAMEWORK` settings in every non-test file of the run (base, production and local settings
alike) and reports:

- when no settings module names a `DEFAULT_PAGINATION_CLASS` (absent, or `None`), at the class name, a concrete list
  view (`ListAPIView`, `ListCreateAPIView`, `ModelViewSet`, `ReadOnlyModelViewSet`, `ListModelMixin`) outside test
  code that assigns no `pagination_class`, itself or through a base view of the run, does not override
  `paginate_queryset()` or `paginator`, hands its `list()`/`get()` to DRF's, and whose rows come from a model table:
  its queryset is a manager chain (`Order.objects.filter(...)`), its serializer's `Meta.model` is a model of the run,
  or its `get_queryset()` returns such a chain. Concrete: no class of the run outside test code derives from it (by
  any name the project imports it under), and its name does not start with `Base` or `Abstract`;
- a pagination class that pages nothing unless the client asks for a page size: DRF's `PageNumberPagination` and
  `CursorPagination` take their page size, and `LimitOffsetPagination` its default limit, from `PAGE_SIZE`, so with
  no `PAGE_SIZE` setting they return every row when the request names none. Reported once at the
  `DEFAULT_PAGINATION_CLASS` value (not at every view it covers), and at a view's `pagination_class = ...` value,
  when it names one of those built-ins or a run class deriving from one that sets no `page_size` (`default_limit`)
  other than `None`, assigns none in `__init__`, and overrides none of `get_page_size()`, `get_limit()` and
  `paginate_queryset()`.

```python
# settings.py: REST_FRAMEWORK = {"PAGE_SIZE": 50}, no DEFAULT_PAGINATION_CLASS
class OrderViewSet(viewsets.ModelViewSet):        # reported: every order in one response
    queryset = Order.objects.select_related("customer")

class CountryList(generics.ListAPIView):          # not reported: unpaginated on purpose
    queryset = Country.objects.all()
    pagination_class = None

REST_FRAMEWORK = {"DEFAULT_PAGINATION_CLASS": "rest_framework.pagination.LimitOffsetPagination"}  # reported: no PAGE_SIZE
```

It leaves alone:

- `pagination_class = None` and any other value, own or inherited, and any class attribute whose name speaks of
  pagination (Tandoor's `pagination_disabled = True`);
- a view (or run base) that overrides `paginate_queryset()` or `paginator`, or whose `list()`, `get()`,
  `get_queryset()` or `filter_queryset()` calls a paginator of its own (label-studio's `paginator(tasks, request)`);
- a view whose own or a run base's `list()`/`get()` calls neither `super().list(...)`, `super().get(...)` nor
  `self.list(...)`;
- a viewset routed only through `as_view({...})` maps that never map a method to `list` and registered with no router.
  The receiver resolves through lib/drf's `drf_cls`, so plane's `UserEndpoint`, which `app/urls/user.py` imports as
  `plane.app.views.UserEndpoint` (a package re-export, under the project's import root `apps/api/`), is left alone;
- a sliced queryset (`Order.objects.all()[:50]`) or a `get_queryset()` that slices;
- views whose rows the rule cannot tie to a model, projects that build `REST_FRAMEWORK` from something other than a
  dict literal or `dict(...)`, runs with no `REST_FRAMEWORK` (a reusable app), and test code;
- a pagination class with a page size of its own, or any `PAGE_SIZE` setting other than `None`.

**Gaps:** whether a table grows with use (predictions, comments, uploads) or stays small by nature (a project's
webhooks and storages, one user's tokens, one task's annotations, a singleton configuration row) is not in the code,
so the rule stays in `log` and off; a list view whose URL is never routed at all is reported; a pagination class set by
a third-party base view or in `__init__` is not seen; an `APIView` or `@api_view` that serialises a whole queryset
itself is not seen; a `PAGE_SIZE` read from configuration is taken as set.

**Corpus** (runs `drf-fix-lwp-<repo>` and `drf-fix-lwp2-<repo>` of the reworked rule, every match read):

| Repository | Commit | Matches (review) | Real | Small by nature |
|---|---|--:|--:|--:|
| HumanSignal/label-studio | `2f42caec` | 20 (11) | 3 | 17 |
| makeplane/plane | `723bb178` | 9 (10) | 3 | 6 |
| paperless-ngx/paperless-ngx | `faa6f09f` | 1 (1) | 0 | 1 |
| inventree/InvenTree | `939df605` | 1 (0) | 1 | 0 |
| wger-project/wger | `f903c66d` | 0 (0) | 0 | 0 |
| babybuddy/babybuddy | `1638cfb0` | 0 (0) | 0 | 0 |

Precision 7 of 31 read matches (23%; the review measured 6 of 22). Real: label-studio
`label_studio/tasks/api.py:1440` (`PredictionAPI`), `users/api.py:128` (`UserAPI`), `data_import/api.py:889`
(`FileUploadListAPI`); plane `apps/api/plane/space/views/issue.py:247`, `:398`, `:603` (a public board's comments,
reactions and votes); and the new clause's InvenTree `src/backend/InvenTree/InvenTree/settings.py:615`:
`LimitOffsetPagination` as the default with no `PAGE_SIZE`, so every InvenTree list returns all rows unless the client
sends `limit`. plane's `UserEndpoint` (the review's wrong match) is gone. label-studio's count grew because its views
now resolve through its import root (`from io_storages.api import ImportStorageListAPI` for
`label_studio/io_storages/api.py`): its ten S3, GCS, Azure, Redis and local-files storage list views are reported in
place of their two base views, all small by nature, as the review judged the bases.

Mode: `log`, off. At this precision it is an inventory a team reads once, not a check.

**Sources:** DRF API guide, [Pagination](https://www.django-rest-framework.org/api-guide/pagination/) (pagination is
off unless a class and a page size are set; `LimitOffsetPagination`'s `default_limit` is `PAGE_SIZE`); OWASP
[Django REST Framework Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Django_REST_Framework_Cheat_Sheet.html),
`DEFAULT_PAGINATION_CLASS`; OWASP API Security Top 10 2023,
[API4 Unrestricted Resource Consumption](https://owasp.org/API-Security/editions/2023/en/0xa4-unrestricted-resource-consumption/).
DRF's system check W001 is the reverse case (`PAGE_SIZE` without a class).

