# Review: the Django REST framework pack (`packs/drf/`)

Reviewer's audit of the 8 rules merged into `packs/drf/`. Nothing here changes a rule, case or `pack.json`; the
verdicts are the output. Every corpus number below comes from runs made for this review on 2026-11-25, not from the
README (whose numbers are the writers' own).

## 1. Summary

Read the rules in this order, by the harm they prevent (`pack.json` lists them alphabetically):
object-permission-skipped, view-without-permission, serializer-exposes-secret-field, declared-field-meta-ignored,
validate-returns-value, page-size-uncapped, related-field-not-prefetched, list-without-pagination.

A match counts as real when a reviewer on that team would ask for the change: the message's claim is true and acting
on it fixes a bug, a leak, or a cost that matters. Where the claim is true but harmless, the rule's section gives both
numbers. Corpus: 13 DRF repositories (§2).

| Rule | Verdict | Proposed mode / on | Matches (repos) | Read | Real | One-line reason |
|---|---|---|--:|--:|--:|---|
| `drf-practice-object-permission-skipped` | rework | steer / on | 3 (1) | 3 | 3 (100%) | Precise, but misses the destructive BOLA shape (`M.objects.filter(pk=pk).delete()` / `.update()`) and flags read-only GETs under `IsOwnerOrReadOnly`. |
| `drf-practice-view-without-permission` | keep | steer / on | 9 (2) | 9 | 9 (100%) | Fires only where settings leave `AllowAny` as the default; every match is reachable anonymously (2 look unintended, 7 are public by design and get a one-line `AllowAny`). |
| `drf-practice-serializer-exposes-secret-field` | rework | log / on | 15 (6) | 15 | 7 (47%) | Real leaks (storage password, webhook HMAC key, Slack token), but name matching also flags invite, export, device and one-time tokens and NLP `tokens`. |
| `drf-practice-declared-field-meta-ignored` | rework | steer / on | 8 (3) | 8 | 8 (100%) | Every match is a dead Meta entry, 5 of them bugs; but the message leads with `read_only=True`, which breaks serializers that write the field on purpose (linkding, InvenTree's admin user edit). |
| `drf-practice-validate-returns-value` | keep | steer / on | 5 (3) | 5 | 5 (100%) | Narrow and DRF-aware; 1 current bug, 4 latent; the fix (`return value`) cannot break code. |
| `drf-practice-page-size-uncapped` | keep | steer / on | 6 (6) | 6 | 6 (100%) | Each match lets `?limit=` / `?page_size=` load a whole table. |
| `drf-practice-related-field-not-prefetched` | rework | log / on | 80 (5) | 80 | 73 (91%) | Real N+1s, but one finding per field (13 on one Tandoor line) and three common fixed shapes it does not see. |
| `drf-practice-list-without-pagination` | rework | log / off | 22 (3) | 22 | 6 (27%) | Mostly small-by-nature lists, as its writer expected; one false positive from an unresolved package re-export. |

Verdicts: keep 3, rework 5, cut 0.

Pack-wide, independent of the verdicts: 11 of the 15 messages are longer than the 200-character cap of
`docs/pack-format.md` ("Message", criterion 6), from 231 to 519 characters as written; the README has two unfilled
placeholders and two "Rule details" sections that are raw pastes of the rule header; and `related-field-not-prefetched`
and the README defer to `drf-practice-method-field-query`, which the pack does not contain (§4).

## 2. Method

- **Corpus.** The DRF repositories of BRIEF.md's table (paperless-ngx, label-studio, plane, InvenTree, pretix,
  addons-server, DefectDojo), linkding (listed as Django, but its API is DRF), the four DRF apps the lead cloned for
  this pack (netbox, Tandoor, wger, babybuddy), and escriptorium, a DRF app already in `MANIFEST.tsv` that no writer
  ran. Commits in §5.
- **Corpus runs**, one per repository with all 8 rules:
  `~/rerail-bench/python-arch/bin/corpus-run <repo> review-drf-all-<repo> packs/drf/rules/drf-practice-*.rerail`.
  All 13 finished inside the 15-minute limit. pretix and DefectDojo did not get a large-pool slot during the first
  pass (the pool stayed full with other reviewers' runs for about two and a half hours) and were rerun once the pool
  grew to 5 slots: DefectDojo 8 findings, pretix 3. Both set `DEFAULT_PERMISSION_CLASSES` and a
  `DEFAULT_PAGINATION_CLASS` (`src/pretix/settings.py:492-498`, `dojo/settings/settings.dist.py:844-857`), so
  view-without-permission and list-without-pagination cannot fire there, and none of the 8 rules' verdicts changed
  with them.
- **Counterexamples**, one small repository per rule under `/tmp/review-drf/<short>/` (`ops`, `vwp`, `sx`, `dfm`,
  `vrv`, `psu`, `lwp`, `rnp`), each run alone:
  `corpus-run /tmp/review-drf/<short> review-drf-<rule>-cx packs/drf/rules/drf-practice-<rule>.rerail`.
- **Rule tests:** `node dist/cli/cli.js rules test packs/drf/rules/<rule>.rerail`: all 8 files pass (16 test blocks).
- **Pack contract:** `npx vitest run test/engine/catalog/packs.test.ts -t "pack drf"`. The first run, on a machine
  loaded with other reviewers' corpus runs, failed 9 of 17 tests at `expect(rule?.status, rule?.reason).toBe("ok")`
  (the tail I kept cut the reason); an identical rerun passed 17 of 17. I could not tell whether load caused the first
  failure; worth one more run on a quiet machine before the pack is registered.
- **Sources:** every URL the rules cite was opened (§4.1).
- **Overlap** was checked against the `python` pack's 538 ports (`packs/python/pack.json`), Ruff's full rule list
  (including DJ, RET), Bandit, pylint/pylint-django, mypy, flake8-django, DRF's own system checks and runtime
  assertions, and the Semgrep registry's DRF rules. django-migration-linter, Squawk, flake8-pytest-style, pydantic's
  mypy plugin and SQLAlchemy's warnings do not read DRF code and overlap none of the 8.

## 3. Rules

### 3.1 `drf-practice-object-permission-skipped`: rework, steer / on

**Value.** Broken object level authorization (OWASP API1), silent, and a trap DRF's docs warn about twice
(`api-guide/permissions.md` line 39: object permissions run "when `.get_object()` is called"; line 267: in custom
views "you'll need to make sure you check the object level permission checks yourself"). Cross-file (permission
classes, base views, the settings default as a dotted string), which no linter models. Few hits on mature code are
expected: the corpus apps mostly call `get_object()`; the rule's value is the new handler that does not.

**Counterexamples** (`/tmp/review-drf/ops`, settings default `IsAuthenticated`):

| # | Shape | Result | Judgement |
|---|---|---|---|
| 1 | `get()` under `IsOwnerOrReadOnly` (returns True for `SAFE_METHODS` before reading `obj`), `get_object_or_404(Note, pk=pk)` | reported, `app/views.py:14` | false positive: the skipped check allows every GET |
| 2 | `delete()` under `IsOwner`: `Note.objects.filter(pk=pk).delete()` | silent | miss: the destructive BOLA shape |
| 3 | `partial_update()` under `IsOwner`: `Note.objects.filter(pk=pk).update(text=...)` | silent | miss |
| 4 | `retrieve()`: `Note.objects.filter(pk=pk)[0]` | silent | miss |
| 5 | permissions from `get_permissions()` returning `[IsOwner()]`; `put()` fetches `Note.objects.get(pk=pk)` | silent | miss (documented gap) |
| 6 | `put()` under `IsOwner`, `Note.objects.get(pk=pk)` | reported, `app/views.py:60` | true positive |

**Corpus.** 3 matches, all in Tandoor, all read, all real: `cookbook/views/api.py:577`, `:607`, `:639`.
`DeleteRelationMixing`'s detail actions `protecting`, `cascading` and `nulling` fetch
`self.queryset.filter(pk=pk, space=request.space).first()`; `RecipeBookViewSet` (permission
`CustomIsOwner | (CustomIsShared & IsReadOnlyDRF)`) mixes it in, so any member of a space lists what references another
member's private recipe book, by id. Low impact (reference metadata), but the check the view declares is skipped.
0 in the other 12 repositories.

**Overlap.** None: Ruff (all families, DJ included), Bandit, pylint/pylint-django, flake8-django and the Semgrep DRF
rules have no model of DRF permissions.

**Text.** Description fine (147 characters, imperative). Both messages fit the cap (195, 193). Two steer-safety
points. `ObjectPermissionSkipped` offers `self.get_object()` first, but `get_object()` also applies
`filter_queryset()` and `lookup_field`, so swapping a pk lookup for it can change which row is found, or 404 when a
filter backend needs a parameter; `self.check_object_permissions(request, obj)` never changes behaviour for an
allowed user and should come first. `FunctionViewObjectPermissionSkipped` suggests
`Perm().has_object_permission(request, None, obj)`; a permission class that reads `view.action` or `view.kwargs` raises
on `None`.

**Verdict: rework, steer / on.**
1. Report `<rows of the view's model>.filter(<URL kwarg>).delete()` and `.update(...)` in a detail handler: the
   act-without-fetch shape (counterexamples 2, 3).
2. Add `.filter(...)[0]` and `.last()` to the fetch shapes (counterexample 4).
3. Leave alone a `get`/`retrieve`/`head` handler when every object-permission class on it returns `True` for
   `SAFE_METHODS` before reading `obj` (DRF's documented `IsOwnerOrReadOnly`; counterexample 1).
4. Lead the message with `check_object_permissions(request, obj)`; in the function-view message, pass the real view or
   compare ownership directly.

### 3.2 `drf-practice-view-without-permission`: keep, steer / on

**Value.** DRF's `DEFAULT_PERMISSION_CLASSES` is `AllowAny` (`api-guide/permissions.md` line 85; OWASP DRF cheat
sheet: change the default). The rule only fires where the project's settings leave that default open, which is the
right scope: the 11 repositories that set a default get nothing.

**Counterexamples** (`/tmp/review-drf/vwp`, settings set only `DEFAULT_AUTHENTICATION_CLASSES`):

| # | Shape | Result | Judgement |
|---|---|---|---|
| 1 | `APIView` whose `initial()` raises `NotAuthenticated` for anonymous users | reported, `app/views.py:10` | false positive (documented gap) |
| 2 | `APIView` overriding `check_permissions()` | reported, `:20` | false positive (documented gap) |
| 3 | `ModelViewSet` with `permission_classes = []` | silent | by design; arguably a miss: an empty list on a writable viewset is `AllowAny` without saying so |
| 4 | plain `ModelViewSet` | reported, `:34` | true positive |
| 5 | `@api_view` imported as `api_view as av` | reported, `:40` | true positive (alias resolved) |
| 6 | `LoginRequiredMixin` + `APIView` | silent | correct: the mixin's `dispatch` refuses anonymous requests |

**Corpus.** 9 matches in 2 repositories, all read. paperless-ngx `src/documents/views.py:4364` (`RemoteVersionView`)
and `:4429` (`ReleaseNotesView`): routed at `/api/remote_version/` and `/api/release_notes/`
(`src/paperless/urls.py:219-226`) with no login wrapper, while every other paperless API view names `IsAuthenticated`;
anonymous callers make the server call api.github.com. Real and likely unintended. addons-server
`src/olympia/accounts/views.py:374`, `:380`, `src/olympia/addons/views.py:1312`, `src/olympia/blocklist/views.py:8`,
`src/olympia/hero/views.py:48`, `:70`, `:80`: login and public catalogue endpoints, public by design. The message is
true for all 9, and its advice for the 7 public ones (`permission_classes = [AllowAny]`) is a one-line,
behaviour-preserving change that records the decision; counted real. 2 of 9 look like accidental exposure.

**Overlap.** None (Ruff, Bandit, pylint-django, flake8-django; DRF has no system check for an open default; the
Semgrep registry's only DRF rule is about throttling).

**Text.** Description fine. The message is 326 characters; `{why}` alone runs to 98 ("the project's REST_FRAMEWORK
settings set no DEFAULT_PERMISSION_CLASSES (DRF's default is AllowAny)"). Shorten `why` to "the project sets no
default permission" and leave the settings alternative to the README. The advice is steer-safe: it offers `AllowAny`
for public endpoints.

**Verdict: keep, steer / on.** Message length only.

### 3.3 `drf-practice-serializer-exposes-secret-field`: rework, log / on

**Value.** OWASP API3 and DRF's "strongly recommended" note on `fields`. The corpus has genuine leaks the rule finds
(storage credentials, a webhook HMAC key, PDF passwords). Detection is by field name, so precision is the name list's.

**Counterexamples** (`/tmp/review-drf/sx`):

| # | Shape | Result | Judgement |
|---|---|---|---|
| 1 | `Document.tokens = JSONField()` (NLP tokens), listed | reported, `app/serializers.py:8` | false positive |
| 2 | `Bucket` with `access_key` (an S3 key id) and `secret_access_key`, `fields = "__all__"` | reported twice, `:14` | `access_key` false positive, `secret_access_key` true positive |
| 3 | `Device.key`, `Device.pin`, listed | silent | miss (documented) |
| 4 | plain `Serializer` declaring `secret_access_key = CharField()` | silent | miss (plain serializers out of scope) |
| 5 | `SerializerMethodField` returning `obj.secret_access_key` | silent | miss |
| 6 | `Share.token` (a public share-link slug), listed | reported, `:39` | false positive |
| 7 | `fields = BUCKET_FIELDS` (module constant) | silent | miss |
| 8 | `extra_kwargs = {"secret_access_key": {"write_only": True}}` | silent | correct |

**Corpus.** 15 matches in 6 repositories (label-studio 4, plane 4, pretix 3, DefectDojo 2, paperless-ngx 1, netbox 1),
all read.

- Real (7): label-studio `label_studio/projects/serializers.py:342` (`ProjectSerializer` lists `task_data_password`,
  the task storage's credentials, returned to every project reader); `label_studio/webhooks/serializers_for_hooks.py:30`
  (`ProjectWebhookSerializer`, `"__all__"`: `task_data_password` and `token`, two matches, posted to the webhook's
  URL); paperless-ngx `src/documents/serialisers.py:3269` (`WorkflowAction.passwords`, the PDF passwords a workflow
  tries); netbox `netbox/extras/api/serializers_/events.py:67` (`Webhook.secret`, the HMAC key); DefectDojo
  `dojo/system_settings/api/serializer.py:9` (`SystemSettingsSerializer`, `"__all__"`: `jira_webhook_secret` and
  `slack_token`, two matches; the view is superuser-only, so the exposure is to every superuser API token holder).
- Not real (8): label-studio `label_studio/organizations/serializers.py:23` (`Organization.token`, the invite-link token,
  which label-studio's own invite endpoint returns too, `organizations/api.py:393-401`); plane `apps/api/plane/app/serializers/api.py:14` (`APIToken.token`, returned
  once by the create view; the list uses `APITokenReadSerializer`), `exporter.py:26` (`ExporterHistory.token`, an
  export job's id), `project.py:217` and `workspace.py:135` (invite tokens, listed to admins who can already invite
  anyone and to the invitee). The README counts plane's two invite tokens as real; I do not. pretix
  `src/pretix/api/serializers/orderchange.py:255` (`OrderPositionChangeSerializer` lists `OrderPosition.secret`, the
  ticket secret; the serializer is input-only: `api/views/order.py:976`, `:1579` and `orderchange.py:376` validate with
  it and never return its data), `src/pretix/api/views/device.py:140` (`DeviceSerializer.api_token`, returned once by
  the unauthenticated `InitializeView` to the device being provisioned) and
  `src/pretix/api/serializers/organizer.py:468` (`Device.initialization_token`, the one-time pairing token shown to
  organizer admins so they can provision the device). Same judgement as the README for pretix.

**Overlap.** Ruff DJ006/DJ007 check Django `ModelForm` only (and are not ported); Bandit B105-B107 are hardcoded
secrets in source, a different hazard. Nothing checks serializer output.

**Text.** Description fine (123). Message 348 characters. "in every response" is false for an input-only serializer
(the README's own pretix `OrderPositionChangeSerializer`); "in its output" is accurate.

**Verdict: rework, log / on.**
1. Drop the plural `tokens` without a suffix and `access_key` without `secret` from the name list: they name NLP
   tokens, stored LLM token data and object-storage key ids more often than secrets (counterexamples 1, 2), and none
   of the corpus's real matches needs them.
2. Message to 200 characters, "output" for "every response".

Precision stays far below 90% (one-time, invite and share tokens are not separable by name), so log.

### 3.4 `drf-practice-declared-field-meta-ignored`: rework, steer / on

**Value.** DRF ignores `read_only_fields` and `extra_kwargs` for declared fields, silently and permanently
(`serializers.md` lines 564, 598; #3533 closed as intended). The corpus shows the cost: InvenTree's user-creation
serializer believes `is_staff` is read-only, and it is not.

**Counterexamples** (`/tmp/review-drf/dfm`):

| # | Shape | Result | Judgement |
|---|---|---|---|
| 1 | `slug = SlugField(**RO)`, module `RO = {"read_only": True}`, `slug` in `read_only_fields` | reported, `app/serializers.py:12` | false positive: the field is read-only, "clients can write it" is false |
| 2 | plain `Serializer` with `Meta.read_only_fields = ["title"]` | silent | miss: a plain serializer ignores its whole Meta |
| 3 | `read_only_fields = READONLY` (module constant) | silent | miss |
| 4 | `extra_kwargs = {"title": {"max_length": 5}}` on a declared `CharField` | reported, `:35` | true positive |
| 5 | `__init__` sets `self.fields["title"].read_only = True`; `title` in `read_only_fields` | reported, `:43` | false claim: the entry is dead, but clients cannot write the field |
| 6 | `author = PrimaryKeyRelatedField(..., default=CurrentUserDefault())` in `read_only_fields` | reported, `:54` | true positive: clients can override the author |

**Corpus.** 8 matches in 3 repositories (InvenTree 6, Tandoor 1, linkding 1), all read.

- InvenTree `src/backend/InvenTree/users/serializers.py:607` (`UserCreateSerializer`, `is_staff`, `is_superuser`,
  declared writable on `ExtendedUserSerializer`, lines 397, 403). The Meta's comment says "Prevent creation of users
  with superuser or staff permissions"; `is_staff` is writable on create, so a staff admin can create staff users.
  `is_superuser` is still guarded by `validate_is_superuser` (line 422). Real (intent broken), 2 matches.
- InvenTree `src/backend/InvenTree/part/serializers.py:577` (`creation_user`): `create()` sets it from `request.user`
  (line 1002), but an update accepts any user. Real (attribution forgery).
- InvenTree `src/backend/InvenTree/common/serializers.py:860` (`filename`): declared
  `CharField(source='basename', required=False)`, and `basename` is a read-only property (`common/models.py:2300`), so
  an update that sends `filename` fails on save. Real (a 500).
- InvenTree `src/backend/InvenTree/users/serializers.py:214` (`UserSerializer`, `username`, `email`): dead entries,
  inherited by `ExtendedUserSerializer` (`Meta(UserSerializer.Meta)`), the serializer of the staff user-edit view
  (`users/api.py:197`), where editing username and email is wanted. Following the message's first advice
  (`read_only=True` on `UserSerializer`'s declared fields) would stop admins from editing them.
- Tandoor `cookbook/serializer.py:843` (`storage`, a writable nested `StorageSerializer` in a
  WritableNestedModelSerializer): real.
- linkding `bookmarks/api/serializers.py:90` (`tag_names`): the field is meant to be written (line 97: "Custom
  tag_names field to allow passing a list of tag names to create/update"); the right fix is to drop the entry.
  Following the message's first advice would break tag assignment through linkding's API.

All 8 are dead Meta entries a reviewer would ask to fix (8 real); 5 are bugs today. 2 of the 8 (linkding, InvenTree's
`UserSerializer`) are writes-intended, where the message's lead advice breaks working code.

**Overlap.** None: no linter reads DRF Meta options, and DRF says nothing at runtime.

**Text.** The description (147) says "read-only and write-only options", but the `extra_kwargs` clause reports any
option (`max_length`, `required`): "Put options on a DRF serializer's declared field itself, not in Meta's
read_only_fields or extra_kwargs; DRF ignores them for declared fields." Messages fit the cap (190, 188), but
`DeclaredReadOnlyIgnored` leads with "Set read_only=True", the change that breaks the two writes-intended matches.

**Verdict: rework, steer / on.**
1. When the serializer, a run base or a run subclass that inherits the entry writes the field (its `create()`,
   `update()`, `validate()` or `validate_<x>()` reads it from `validated_data`/`attrs`, or a subclass serves an update
   view), use a second message: "`x` is written on purpose here; drop it from read_only_fields". Otherwise keep
   `read_only=True`, worded "if clients must not write it".
2. Treat `**NAME` (a module-level dict literal with `"read_only": True`) as `read_only=True`, and an `__init__` that
   assigns `self.fields["x"].read_only` as applying the option (counterexamples 1, 5).

### 3.5 `drf-practice-validate-returns-value`: keep, steer / on

**Value.** DRF stores a field hook's return value as the field's value (`serializers.py` line 515) and asserts that
`validate()` returned something (lines 460, 652). The silent-None case is a known trap (SO 65420838, TestDriven).
Narrow: only hooks DRF calls.

**Counterexamples** (`/tmp/review-drf/vrv`):

| # | Shape | Result | Judgement |
|---|---|---|---|
| 1 | `validate_email` ending in `reject("bad")`, a module function whose body only raises | reported, `app/serializers.py:18` | false positive |
| 2 | `validate_code` ending in `self.fail("invalid")` (`Field.fail` raises `ValidationError`) | reported, `:24` | false positive |
| 3 | `validate()` with `return None` on one path | silent | miss: DRF's assertion fails, a 500 (documented, deliberate) |
| 4 | `validate_name` on a plain mixin | silent | miss (documented) |
| 5 | `validate_age` that assigns `value = int(value)` and falls off | reported, `:38` | true positive |

**Corpus.** 5 matches in 3 repositories, all read. label-studio `label_studio/projects/serializers.py:583`
(`validate_filter`: an unknown filter becomes None instead of a 400): a bug today. paperless-ngx
`src/documents/serialisers.py:2407` (`validate_created` returns only for datetimes): latent. escriptorium
`app/apps/api/serializers.py:1181` (`validate_iiif_uri`), `:1189` (`validate_mets_uri`), `:1199`
(`validate_upload_file`): hooks that never return; they set `self.file`, and the three fields become None in
`validated_data`, which `validate()` and `process()` only test with `'x' in data`: latent. A reviewer asks for
`return value` in all 5, and that change cannot break working code (the field keeps the value it arrived with), so
5 of 5 real, 1 of 5 a bug today.

**Overlap.** Ruff RET503 (not ported by the `python` pack, not in Ruff's default set) and pylint R1710 flag the two
corpus hooks that return on some paths (paperless-ngx, label-studio) and none of escriptorium's three, which never
return; mypy only with an annotated return type. The rule adds the never-returns shape and the DRF scoping (helpers
named `validate_*` that DRF never calls stay silent).

**Text.** The description does not start with a verb (criterion 7): "Return the validated value from every path of a
DRF serializer's validate_<field>() and validate() that does not raise." Messages are 330, 254, 254 and 231
characters.

**Verdict: keep, steer / on.** Not blocking: count `self.fail(...)` and calls of a run function whose body only raises
as raising (counterexamples 1, 2), and report `return None` in `validate()`, which DRF rejects every time (unlike in a
field hook).

### 3.6 `drf-practice-page-size-uncapped`: keep, steer / on

**Value.** `max_limit` and `max_page_size` default to None (`pagination.md` lines 199, 247); OWASP API4 names records
per page. One request serialises a table.

**Counterexamples** (`/tmp/review-drf/psu`):

| # | Shape | Result | Judgement |
|---|---|---|---|
| 1 | `LimitOffsetPagination` subclass whose `paginate_queryset()` clamps with `min(self.get_limit(request) or 50, 500)` | reported, `app/pagination.py:8` | false positive |
| 2 | built-in `LimitOffsetPagination()` instantiated in an `@api_view` | silent | miss |
| 3 | subclass with `max_limit = 10 ** 9` | silent | by design (documented: any non-None cap) |

**Corpus.** 6 matches in 6 repositories, all read, all real: DefectDojo `dojo/settings/settings.dist.py:856`,
babybuddy `babybuddy/settings/base.py:362`, InvenTree
`src/backend/InvenTree/InvenTree/settings.py:615`, linkding `bookmarks/settings/base.py:151` (built-in
`LimitOffsetPagination` as the default); label-studio `label_studio/organizations/api.py:86`
(`OrganizationMemberListPagination`: `page_size` from the client, no cap); netbox `netbox/netbox/api/pagination.py:200`
(`LimitOffsetListPagination`, superuser-only lists already in memory: low impact). InvenTree also shows a shape no rule
covers: its default is `LimitOffsetPagination` with no `PAGE_SIZE`, and `LimitOffsetPagination` paginates nothing when
the client sends no `limit` and `default_limit` is None, so every InvenTree list is unpaginated unless the client asks;
`max_limit` does not change that (§4.2).

**Overlap.** None; DRF's check W001 is the reverse case (`PAGE_SIZE` without a class).

**Text.** Description fine. Messages 353, 285 and 329 characters. Steer-safe: each says a client asking for more gets
the cap.

**Verdict: keep, steer / on.** Not blocking: count a built-in class instantiated in code as a use, and accept a clamp
in an overridden `paginate_queryset()`.

### 3.7 `drf-practice-related-field-not-prefetched`: rework, log / on

**Value.** The N+1 behind most slow DRF list endpoints (DRF generic views, "Avoiding N+1 Queries"; Haki Benita). It
joins view, serializer and model across three files: the pack's most cross-file rule, and no static linter does it.

**Counterexamples** (`/tmp/review-drf/rnp`):

| # | Shape | Result | Judgement |
|---|---|---|---|
| 1 | `select_related(*RELS).prefetch_related(*M2M)` with module-level lists | reported twice, `app/views.py:12` | false positive |
| 2 | `list()` overridden to serialise with a lighter serializer | reported twice, `:17` | false positive |
| 3 | `paginate_queryset()` calling `prefetch_related_objects(page, "author", "tags")` | reported twice, `:24` | false positive |
| 4 | `SerializerMethodField` returning `obj.author.name` | silent | miss (deferred to `drf-practice-method-field-query`, which does not exist) |
| 5 | plain `queryset = Book.objects.all()` | reported twice, `:38` | true positive |

**Corpus.** 80 matches in 5 repositories (Tandoor 45, paperless-ngx 21, InvenTree 7, DefectDojo 5, wger 2), on 27
views; every match read at view level, with the serializer field and the queryset.

- Tandoor (45, 17 views): real 38, e.g. `cookbook/views/api.py:1690` (`StepViewSet`: nested `ingredients` many and
  `file`), `:2152` (`UnitConversionViewSet`, three nested FKs), `:879` (`InventoryEntryViewSet`), `:705`
  (`SpaceViewSet`: 13 nested `UserFile`/`AiProvider` FKs and an M2M, 13 findings on one line). Not real 7: `:795`
  (`UserPreferenceViewSet`, 2) and `:818` (`SearchPreferenceViewSet`, 5) list one row per user
  (`filter(user=self.request.user)`), so "per row" means once.
- paperless-ngx (21, 2 views): real. `src/documents/views.py:5236` `WorkflowActionViewSet` prefetches the six
  `assign_*` M2Ms but none of the `remove_*` ones nor `email`/`webhook`; `:5218` `WorkflowTriggerViewSet` loads none of
  its 9 M2M filters. Small admin tables, but a genuine partial omission.
- InvenTree (7, 4 views): real. `common/api.py:1663`, `importer/api.py:92`, `part/api.py:1518` (`PartBriefSerializer`
  on every substitute), `users/api.py:547` (three nested users per token).
- wger (2): real. `wger/exercises/api/views.py:354`, `:417` (`ReadOnlyField(source='exercise.uuid')`).
- DefectDojo (5, 2 views): real. `dojo/notes/api/views.py:30` (`NotesViewSet.get_queryset()` returns
  `Notes.objects.all().order_by("id")`, and `NoteSerializer` nests `author`, `editor`, `note_type` and the `history`
  many: 4 findings); `dojo/user/api/views.py:98` (`UserContactInfoViewSet`, nested `UserSerializer` on `user`). The
  latter's `PrefetchListMixin.list()` (`dojo/api_v2/prefetch/mixins.py:8`) still serialises each row with the
  view's serializer; its `?prefetch=` only adds a side table. Both views are superuser-only and paginated by 25.

Precision 73 of 80 (91%). The noise is volume: one finding per field, so one view's fix can be 13 findings. plane
gives 0 because its `BaseViewSet.get_queryset()` returns `self.model.objects.all()` (`app/views/base.py:84`), a root
the rule does not resolve; I did not trace why label-studio gives 0.

**Overlap.** None static; runtime tools (nplusone, django-zen-queries) catch it in tests. The django pack's
`select-related-in-loop` covers explicit loops only.

**Text.** Description fine (158). Message 337 characters; it names the fix and when to leave it.

**Verdict: rework, log / on.**
1. One finding per view and serializer, listing every relation (`select_related("a", "b")`, `prefetch_related("c")`).
2. Follow `*NAME` splats of module-level string lists and tuples in `select_related`/`prefetch_related`.
3. Leave alone a view whose own or a run base's `list()` serialises with another serializer class, or whose `list()` or
   `paginate_queryset()` calls `prefetch_related_objects`.
4. Build `method-field-query` or drop the references to it (rule header line 26, README).

Stay in log: the 90% is one view away, and in steer the fix would pull agents into querysets on unrelated edits.

### 3.8 `drf-practice-list-without-pagination`: rework, log / off

**Value.** A real hazard (OWASP API4; DRF paginates nothing by default), but whether a table grows is not in the
code, as the writer says. Its real matches are good ones (label-studio's prediction list).

**Counterexamples** (`/tmp/review-drf/lwp`, settings without a pagination class):

| # | Shape | Result | Judgement |
|---|---|---|---|
| 1 | `ListAPIView` over `Token.objects.filter(user=self.request.user)` | reported, `app/views.py:9` | false positive in practice (a handful of rows per user) |
| 2 | `APIView.get` returning `EventSerializer(Event.objects.all(), many=True).data` | silent | miss (documented gap) |
| 3 | `@api_view` doing the same | silent | miss |
| 4 | `ReadOnlyModelViewSet` over `Event.objects.all()` | reported, `:25` | true positive |

**Corpus.** 22 matches in 3 repositories (label-studio 11, plane 10, paperless-ngx 1), all read.

- Real (6): label-studio `label_studio/tasks/api.py:1440` (`PredictionAPI`: every accessible prediction, filters
  optional), `label_studio/users/api.py:128` (`UserAPI`: every user of the organization),
  `label_studio/data_import/api.py:889` (`FileUploadListAPI`); plane `apps/api/plane/space/views/issue.py:247`, `:398`,
  `:603` (comments, reactions and votes of a public board).
- Small by nature (15): paperless-ngx `src/paperless/views.py:412` (one configuration row); label-studio
  `label_studio/data_export/api.py:372`, `data_manager/api.py:199`, `io_storages/api.py:38`, `:85`, `ml/api.py:69`,
  `organizations/api.py:59`, `tasks/api.py:1241` (one user's drafts on one task), `webhooks/api.py:58`; plane
  `apps/api/plane/app/views/analytic/base.py:183`, `project/invite.py:39`, `:132`, `workspace/base.py:54`, `:325`,
  `apps/api/plane/space/views/issue.py:494`.
- Wrong (1): plane `apps/api/plane/app/views/user/base.py:65` (`UserEndpoint`) is never routed to `list` (its
  `as_view` maps in `app/urls/user.py` name `retrieve`, `partial_update`, `deactivate`, ...). The rule's
  `lwp_list_unrouted` exists for this view and the README says it is left alone, but `app/urls/user.py` imports the
  class through the `plane.app.views` package re-export, which the rule does not resolve to the class.

Precision 6 of 22 (27%).

**Overlap.** None; DRF's W001 is the reverse case.

**Text.** Description fine (156). The message is 519 characters, the pack's longest.

**Verdict: rework, log / off.**
1. Resolve `as_view`/`register` receivers through package re-exports (or by a unique class name), so the documented
   plane exemption works.
2. Message to 200 characters.
3. Add the InvenTree shape: a `DEFAULT_PAGINATION_CLASS` with no `PAGE_SIZE`, whose class sets no `page_size` or
   `default_limit`, paginates nothing by default (§3.6).

Off by default: at 27% it is an inventory a team reads once, not a check.

## 4. Pack-level findings

### 4.1 Sources

Opened on 2026-11-25; every quotation a rule leans on is where the README says:

- DRF `docs/api-guide/serializers.md` (master): line 564 (`read_only_fields` "will not apply" to a declared field),
  line 598 (`extra_kwargs` "will be ignored"), line 181 (`validate_<field_name>` "should return the validated value").
- DRF `rest_framework/serializers.py` (master): line 515 `validated_value = validate_method(validated_value)`; lines
  460 and 652 `assert value is not None, '.validate() should return the validated data'`.
- DRF `docs/api-guide/permissions.md`: lines 39, 222, 267 (object permissions run in `get_object()`; custom and
  function views must call `check_object_permissions`), line 85 (`AllowAny` default).
- DRF `docs/api-guide/pagination.md`: lines 199 and 247 (`max_page_size`, `max_limit` default None).
- encode/django-rest-framework issues #3533 and #3460: exist, titles as cited.
- Stack Overflow 54783424 (score 10), 68946823 (5), 65420838 (2): exist, titles as cited (checked through the Stack
  Exchange API; the site answers scripts with a bot wall).
- TestDriven.io "Permissions in Django REST Framework", Haki Benita "Improve Serialization Performance in Django Rest
  Framework", the OWASP DRF cheat sheet: resolve, titles as cited.

Every rule has at least two independent sources; the thinnest are the two pagination rules (DRF docs and OWASP).

### 4.2 Missing rules

- **`drf-practice-method-field-query`** (rank 9 of the lead's build list; not built). `related-field-not-prefetched`
  skips every `SerializerMethodField`, and its header (line 26) and README name this rule as the one that covers them.
  The 13 repositories have 346 `SerializerMethodField(` calls; counterexample 4 of §3.7 shows the N+1 left behind.
- **A default pagination class with nothing to paginate by.** InvenTree
  (`src/backend/InvenTree/InvenTree/settings.py:606-625`) sets `LimitOffsetPagination` and no `PAGE_SIZE`, so DRF returns
  every row unless the client sends `limit`. `list-without-pagination` treats any default class as paging and
  `page-size-uncapped` only asks for `max_limit`, so the corpus's broadest unbounded-list setting is reported by neither
  as such. One more clause of `list-without-pagination`, not a new rule.
- **`return None` in `validate()`**: one clause of `validate-returns-value` (§3.5).
- Checked, not worth a rule: `ordering_fields = "__all__"` (DRF warns it can let clients order by sensitive fields):
  10 uses, all wger (`wger/core/api/views.py:284` and 3 more, `wger/exercises/api/views.py:97` and 5 more), all on
  public reference tables. Writable `is_staff`/`is_superuser` beyond the declared-field case: InvenTree
  (`users/serializers.py:422`) and DefectDojo (`dojo/user/api/serializer.py:170-178`) guard them by hand in validation,
  so a name-based rule would mostly flag guarded code.

### 4.3 Rules in the wrong pack

None. `validate-returns-value` is the DRF member of a family (the django pack's `clean-field-returns-value`, the
pydantic pack's `validator-returns-value`); its flow code is a renamed copy of the django rule's (`vrv_*` = `cfr_*`),
which core's library could own once.

### 4.4 The library (`rules/lib/drf.rerail`)

Good: one documented interface (every relation listed with its meaning), DRF classes resolved through the run's class
graph plus a short list of third-party bases, settings read in all their forms (dict literal, `dict()`, subscript,
`.update()`), and an `e.g.` block that doubles as a spec. Weak:

- `rules/lib/django.rerail` is a 484-line verbatim copy of the django pack's library; the two will drift. The pack
  format lets a rule include only core's library and files beside it, so the fix is a shared Django library in core.
- Import-root suffix resolution is written three times: `drf_ref_class` (lib), `ops_mq`/`ops_ser`
  (object-permission), `sx_resolve` (secret-field), and `drf_view_model` misses what they resolve (the
  object-permission README says so). One resolver in the lib would serve all.
- Node-returning variants of `drf_str_elem` are re-implemented per rule (`sx_elem`, `dfm_elem_in`), and the
  `extra_kwargs` write-only parse exists twice (`sx_write_only_kwargs`, `rnp_wo`).
- Package re-exports (`from plane.app.views import UserEndpoint`) are not followed (§3.8): a lib-level gap that
  affects every rule matching a class by the name another module imports it under.

### 4.5 README and pack text

- `README.md` lines 89 and 91 are unfilled placeholders (`PRETIX_NETBOX_ROWS`, `PRECISION_LINE`) in the
  declared-field section.
- The `list-without-pagination` (lines 128-169) and `related-field-not-prefetched` (lines 341-395) sections are raw
  pastes of the rule header: no Markdown lists, examples without their indentation, and neither has corpus numbers or
  sources, which "Rule details" asks for.
- The README says plane's `UserEndpoint` is left alone by `list-without-pagination`; on this run it is reported
  (§3.8).
- 11 of 15 messages exceed 200 characters. Descriptions all fit; `validate-returns-value`'s is not imperative; four
  descriptions say "DRF" and four "Django REST framework" (criterion 7, one voice).
- `dimensions.json` proposes new hazard ids in the `<area>.<name>` form (`api.unpaginated-list`,
  `api.unbounded-page-size`, `auth.object-permission-skipped`, `auth.endpoint-without-auth`,
  `validation.hook-returns-none`); `auth.endpoint-without-auth` is close to BRIEF's `auth.mutation-without-auth`, and
  the coordinator should pick one.

### 4.6 Can the pack be pitched on its own?

Yes, as "the DRF review checklist". A team gets the authorization pair (object-permission-skipped,
view-without-permission), the Meta trap that left InvenTree's `is_staff` writable, and the validate hook: all steering,
all precise on this corpus, all cross-file in a way no linter is. The performance half (the N+1 rule and the two
pagination rules) is useful as a log-mode inventory, not as a steer. What would sell it harder: method-field-query and
the InvenTree pagination clause, both in §4.2.

## 5. Corpus repositories and commits

From `~/rerail-bench/python-arch/corpus/MANIFEST.tsv`; no repository was added for this review.

| Directory | Repository | Commit | Run |
|---|---|---|---|
| `sissbruecker__linkding` | sissbruecker/linkding | `27b7303baf41` | `review-drf-all-sissbruecker__linkding` |
| `babybuddy__babybuddy` | babybuddy/babybuddy | `1638cfb02e26` | `review-drf-all-babybuddy__babybuddy` |
| `TandoorRecipes__recipes` | TandoorRecipes/recipes | `22b5841a7828` | `review-drf-all-TandoorRecipes__recipes` |
| `paperless-ngx__paperless-ngx` | paperless-ngx/paperless-ngx | `faa6f09f289b` | `review-drf-all-paperless-ngx__paperless-ngx` |
| `wger-project__wger` | wger-project/wger | `f903c66d3c0c` | `review-drf-all-wger-project__wger` |
| `UB-Mannheim__escriptorium` | UB-Mannheim/escriptorium | `7e06bd31732c` | `review-drf-all-UB-Mannheim__escriptorium` |
| `HumanSignal__label-studio` | HumanSignal/label-studio | `2f42caecbf6c` | `review-drf-all-HumanSignal__label-studio` |
| `inventree__InvenTree` | inventree/InvenTree | `939df605f43d` | `review-drf-all-inventree__InvenTree` |
| `makeplane__plane` | makeplane/plane | `723bb178f97b` | `review-drf-all-makeplane__plane` |
| `netbox-community__netbox` | netbox-community/netbox | `69978099811c` | `review-drf-all-netbox-community__netbox` |
| `mozilla__addons-server` | mozilla/addons-server | `7e3374fe5383` | `review-drf-all-mozilla__addons-server` |
| `pretix__pretix` | pretix/pretix | `40ea9097d0b2` | `review-drf-all-pretix__pretix` |
| `DefectDojo__django-DefectDojo` | DefectDojo/django-DefectDojo | `cd0a4483a224` | `review-drf-all-DefectDojo__django-DefectDojo` |

Counterexample repositories: `/tmp/review-drf/{ops,vwp,sx,dfm,vrv,psu,lwp,rnp}`, runs `review-drf-<rule>-cx`.

## Applied

Applied on 2026-11-25; the review text above is unchanged. Rules are listed in the reading order of §1, in `pack.json`
and the README alike. Every message is at most 200 characters as written, every description one imperative sentence
that says "DRF".

- **Library (`rules/lib/drf.rerail`, §4.4).** Root cause behind several findings: a class named through the project's
  import root (`plane.app.views.X` for `apps/api/plane/app/views/...`) or a package re-export did not resolve, and three
  rules patched it by suffix. One resolver now, `drf_cls(q, c)` / `drf_cls_q`: exact qualname, import root and
  re-exports, for the names `drf_cls_ask` lists (syntax only; rules extend it). The class graph (`drf_lib_q`,
  `drf_ancestor`, `drf_view_serializer`, `drf_meta_model`, `drf_meta_opt` through `Base.Meta`, `drf_nested`,
  `drf_ref_class`, `drf_obj_q`) and the new `drf_qs_model` and `drf_sub` (subclasses by any import name) use it;
  `ops_mq`/`ops_ser`, `sx_resolve` and four rules' exact-qualname subclass checks are gone. Shared relations replace the
  per-rule copies: `drf_str_at` (`sx_elem`, `dfm_elem_in` deleted), `drf_extra_kw` and `drf_dict_entry`
  (`sx_write_only_kwargs`, `rnp_wo` deleted), `drf_const` (module constants in Meta options and splats), `drf_kw`
  through `**NAME`, `drf_related_load` through `*NAME`. Not done: `lib/django.rerail` is still a copy of the django
  pack's library (a shared Django library belongs in core); relative imports in directories without `__init__.py`
  (plane's `from .. import BaseViewSet`) are a core name-resolution gap.
- **`object-permission-skipped`** (steer, on; 3 of 3). Reports `<queryset>.filter(<URL kwarg>).delete()` / `.update()`
  (managers, `self.queryset`, `self.get_queryset()`, local aliases) and the fetch ends `.last()`, `.latest()`,
  `.earliest()`, `[0]` and a filtered `.get()`; leaves safe-method handlers alone when every object permission returns
  True for `SAFE_METHODS` first, read from the `has_object_permission` body (aliases, constants, bases), never the class
  name; reads a `get_permissions()` with one return of instances (counterexample 5). Messages lead with
  `check_object_permissions(request, obj)`; the function-view message compares ownership. Corpus: Tandoor's 3, all
  real; 0 in label-studio and paperless-ngx. Counterexamples: 1 silent, 2-6 reported.
- **`view-without-permission`** (steer, on; 9 of 9). Message 175 characters with the shortened `why`. A base view
  imported through the import root is no longer taken for a third-party base, and subclasses are found by any import
  name. paperless-ngx 2 and addons-server 7, unchanged.
- **`serializer-exposes-secret-field`** (log, off; 7 of 15). Bare `tokens` and `access_key` without `secret` are off
  the name list; the message says "in its output"; added a `SerializerMethodField` returning `obj.<secret>` and
  `fields` through module constants (counterexamples 5, 7); `extra_kwargs` write-only through `drf_extra_kw`. Same
  corpus matches as the review; counterexamples 1 and 2's `access_key` silent. Detection stays by name, so invite,
  share, export and one-time tokens are still reported (counterexample 6), hence off.
- **`declared-field-meta-ignored`** (steer, on; 8 of 8). A second message, "written on purpose here; drop it from
  read_only_fields", when a serializer carrying the entry reads the field in `create`/`update`/`validate`/`save`/
  `to_internal_value`, has `validate_<x>`, or its field class parses input; otherwise a two-option message, never
  `read_only=True` alone. `**RO` and an `__init__`/`get_fields` setting `read_only` silence a match; plain Serializers
  and `read_only_fields` constants are reported. Item 1's "subclass whose Meta inherits the entry" clause was not
  built: it is also the shape of a forgeable field, and the rule cannot confirm the write there. linkding and
  InvenTree's `email` now get "drop the entry".
- **`validate-returns-value`** (steer, on; 5 of 5). Imperative description; messages 167-187 characters;
  `self.fail(...)` and calls of run functions or methods whose every path raises end a path (counterexamples 1, 2);
  `return None` in `validate()` is reported (counterexample 3).
- **`page-size-uncapped`** (steer, on; 6 of 6). Messages 171-190 characters; a built-in `LimitOffsetPagination()`
  instantiated in code is reported (counterexample 2); a `paginate_queryset()` that calls `min()` or raises caps the
  class (counterexample 1).
- **`related-field-not-prefetched`** (log, on; 31 of 31, was 73 of 80 per field). One finding per view and serializer
  listing every missing relation; `*NAME` splats (lib); left alone: a `list()` that does not serialise through
  `get_serializer`/`super().list()`, `prefetch_related_objects` in `list`/`paginate_queryset`/`get_queryset`/
  `filter_queryset`, unreadable load arguments, one-row querysets (a filter on a OneToOne, unique or primary-key field,
  read from the model: Tandoor's 7); `self.model.objects` followed; every `method-field-query` reference removed.
  escriptorium gains 6 real matches, its models now resolved through its import root.
- **`list-without-pagination`** (log, off; 7 of 31). `as_view`/`register` receivers resolve through `drf_cls`, so
  plane's `UserEndpoint` is silent; message 181 characters; new clause for a pagination class with no page size and no
  `PAGE_SIZE` (InvenTree, §4.2), once at the setting and at a view's `pagination_class`. label-studio grew from 11 to 20:
  its storage list views now resolve through its import root and are reported in place of their two bases, all small
  by nature.
- **README (§4.5).** Placeholders and raw-pasted sections replaced, numbers re-measured, the `UserEndpoint` claim now
  true, rules table and order match `pack.json`.
- **Out of scope, for the coordinator:** `drf-practice-method-field-query` (§4.2) is not built; the hazard id choice
  `auth.endpoint-without-auth` vs `auth.mutation-without-auth` (§4.5) is left open.
- **Verification.** All 8 rule test files pass (`node dist/cli/cli.js rules test`, after the last rule and library
  change). The pack contract was run three times on a machine at load average 43-78: every time the engine's 30 s
  describe deadline refused all 8 rules; with `RERAIL_RR_DESCRIBE_DEADLINE_MS=600000` every rule and check passed except
  check 1, which hit vitest's 600 s timeout. Describing the 8 drf rules alone took 28.6 s against 86.7 s for 8 django
  rules under the same load. The contract still needs one run on a quiet machine. A final run of all 8 rules on the
  Junction fleet (`drf-final-<repo>`) over the 9 corpus repositories under 1,000 files (linkding, babybuddy,
  escriptorium, Tandoor, paperless-ngx, InvenTree, wger, label-studio, plane) gave the README's numbers. The four
  repositories of 1,000 files or more are not in it: earlier per-rule reruns cover netbox and DefectDojo
  (secret-field) and addons-server (view-without-permission), with the review's matches; pretix (secret-field) and
  DefectDojo (page-size) were not re-run, and their rows are the review's numbers.
