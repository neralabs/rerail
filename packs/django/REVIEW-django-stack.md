# Review: the Django pack's six stack rules

Reviewer's audit of the six rules added by the Django stack batch (`NOTES-django-stack.md`): what each catches, what it
misses, what it reports that it should not, how it reads on the corpus, and what to do with it. Nothing in `rules/`,
`cases/` or `pack.json` was changed; the verdicts below are the output.

## Summary

Corpus: the 10 Django repositories of BRIEF.md, all six rules in one run each; every match read (pretix's 78 one-line
`id` → BigAutoField AlterFields judged by their table). Precision is real matches over read matches.

| Rule | Verdict | Proposed mode / on | Corpus matches (repos) | Read precision | Reason |
|---|---|---|--:|--:|---|
| `django-practice-settings-env-fails-open` | rework | steer / on | 6 (5) | 6/6 (100%) | Right and valuable; messages are 344+ characters with one fix list for three settings; misses `bool("False")`, a str DEBUG fallback and `!= "0"` reads |
| `django-practice-addfield-default-runs-once` | rework | log until reworked, then steer / on | 8 (3) | 6/8 (75%) | Finds shared secrets (pretix, saleor); its `null=True` exemption is wrong per Django's schema editor; "use db_default" can rewrite the table; reports squashes and cosmetic `random` defaults |
| `django-practice-bulk-ignore-conflicts-used` | rework | log until reworked, then steer / on | 13 (3) | 9/13 (69%) | Real bugs (plane mails unstored tokens), but reports keys the code sets and models where nothing can conflict; messages 343/321 characters |
| `django-practice-runpython-with-schema-change` | rework (text) | log / off | 85 (7) | 60/85 (71%) | Sound analysis, one real pending-trigger failure (DefectDojo); size-blind, so below the 90% bar for on; messages 390/306 characters; misses the `SET CONSTRAINTS ALL IMMEDIATE` fix |
| `django-practice-alterfield-rewrites-table` | rework | log / off | 153 (8) | 112/153 (73%) | Every reported change does rewrite; size-blind (paperless: 22 of 23 on configuration tables), no one-row or MySQL exemption; message 336 characters |
| `django-practice-runpython-live-model` | keep | steer / on | 8 (3) | 8/8 (100%) | Catches what django-migration-linter cannot (helpers in other modules); one false-positive shape (a None guard passed by position) |

Verdicts: 1 keep, 5 rework, 0 cut.

## How this review was run

- Each rule's own `.test` block: `nice -n 19 node dist/cli/cli.js rules test packs/django/rules/<rule>.rerail`. All six
  pass.
- Counterexamples written by the reviewer, one small repository per rule under `/tmp/review-django/cx-*`, run with
  `~/rerail-bench/python-arch/bin/corpus-run /tmp/review-django/cx-<x> review-django-<rule>-cx packs/django/rules/<rule>.rerail`.
  Each counterexample line carries a `CX-..` label; the result is recorded per rule below.
- Corpus: all six rules in one run per repository,
  `corpus-run <repo> review-django-six-<repo> packs/django/rules/django-practice-{six rules}.rerail`, over the 10 Django
  repositories of BRIEF.md's corpus table (commits in the last section). Findings are in
  `~/rerail-bench/python-arch/runs/review-django-six-<repo>/findings.tsv`.
- Every match was read in its file, except pretix's 78 one-line `id` → BigAutoField AlterFields, judged by table.
  "Real" is judged per rule (the criterion is given in each section); the writers' README numbers were not reused.
- Overlap: django-migration-linter read at `main` `0dab899b` (`src/django_migration_linter/sql_analyser/base.py`,
  `postgresql.py`, `migration_linter.py`), pylint-django at `f5fece4f` (`checkers/migrations.py`), Django 5.2's
  `django/db/backends/base/schema.py` and `django/db/migrations/migration.py`, Squawk's rule list, the `python` pack's
  ports (`packs/python/pack.json`), Ruff's DJ/S/PLW rules.

## Reading order

`pack.json` lists the rules alphabetically. Read them in this order, by what a miss costs and how sure each finding is:

1. `django-practice-settings-env-fails-open`: a breach on one forgetful deploy; near-perfect precision.
2. `django-practice-addfield-default-runs-once`: a shared secret or a failed deploy; data that stays wrong after the
   migration.
3. `django-practice-bulk-ignore-conflicts-used`: silent wrong results in request code (plane mails tokens never
   stored).
4. `django-practice-runpython-with-schema-change`: a deploy that fails on production data, or a table locked through
   a backfill.
5. `django-practice-alterfield-rewrites-table`: a table locked for a rewrite; right about the rewrite, blind to size.
6. `django-practice-runpython-live-model`: a migration that breaks later, on a database that runs it late.

The sections below follow that order.

## 1. `django-practice-settings-env-fails-open`

**Value.** A settings module whose environment read falls back to `DEBUG=True`, `"*"` in `ALLOWED_HOSTS` or a
literal `SECRET_KEY` is one missing variable away from debug pages, Host-header poisoning or forgeable sessions. The
sources hold: the Django settings docs ("Never deploy a site into production with DEBUG turned on"), the deployment
checklist (load the secret key from the environment instead of the settings module), CWE-1188. It is a shape a
security reviewer flags every time, and the corpus has it in 5 of 10 repositories.

**Correctness.** The logic resolves project helpers across files (`envbool`, `get_bool_env`, `get_env_list`), the
`Env` schema, decouple, `or` fallbacks and comparisons, and leaves development modules and overridden base modules
alone. Counterexamples (`/tmp/review-django/cx-settings`):

| Label | Shape | Should | Did |
|---|---|---|---|
| CX-E1 | `DEBUG = os.getenv("DEBUG", "False")` (a str: always truthy) | report | silent (miss) |
| CX-E3 | `DEBUG = bool(os.environ.get("DEBUG", "False"))` (`bool("False")` is True) | report | silent (miss) |
| CX-E4 | `DEBUG = os.environ.get("DJANGO_DEBUG") != "0"` (unset: `None != "0"`, True) | report | silent (miss) |
| CX-E2 | `SECRET_KEY = os.environ.get("SECRET_KEY", "insecure-dev-key")` in `settings/base.py`; `production.py` star-imports it and raises `ImproperlyConfigured` when the key is still the default | silent | reported (false positive) |
| CX-E5 | `SECRET_KEY = os.environ.get("SECRET_KEY", "x")` followed by `assert SECRET_KEY != "x" or DEBUG` | silent | reported (false positive, minor: `-O` strips asserts) |
| CX-E6 | `DEBUG = os.environ.get("APP_ENV", "development") == "development"` | report | reported |

The three misses are fail-open DEBUG reads the rule's own definition covers ("DEBUG true when the variable is
unset"): `fo_on` reads `bool()` and `int()` of a fallback through `fo_truthy`, which treats the string `"False"` as
false, and it has no clause for a read without a fallback compared with `!=`. The NOTES dropped
`debug-env-string-truthy` for having 1 real match; its fail-open half belongs here, and the README's "Gaps" paragraph
attributes CX-E1 to "Ruff's PLW1508 family", which is wrong: PLW1508 flags only a non-`str` default, and `"False"` is a
str. The false positive CX-E2 is the guard pattern the rule already honours in the same module (`fo_checked`) but not
in a module that star-imports the base (`fo_importer` is used only for overrides).

**Corpus.** 6 matches in 5 repositories, all read, 6 real (100%); linkding, paperless-ngx, InvenTree, pretix and
DefectDojo have none:

- healthchecks `hc/settings.py:69` `SECRET_KEY = envsecret("SECRET_KEY", "---")` and `:71`
  `DEBUG = envbool("DEBUG", "True")`: real; a self-hosted deploy without `.env` runs with debug on and a public key.
- label-studio `label_studio/core/settings/base.py:335` `ALLOWED_HOSTS = get_env_list('ALLOWED_HOSTS', default=['*'])`:
  real; no deployed module overrides it.
- plane `apps/api/plane/settings/common.py:95` `ALLOWED_HOSTS = os.environ.get("ALLOWED_HOSTS", "*").split(",")`:
  real.
- addons-server `src/olympia/lib/settings_base.py:349`
  `SECRET_KEY = env('SECRET_KEY', default='this-is-a-dummy-key-and-its-overridden-for-prod-servers')`: real; the
  default's own text says production relies on the variable.
- saleor `saleor/settings.py:86` `DEBUG = get_bool_from_env("DEBUG", True)`: real.

**Overlap.** None that covers it. `manage.py check --deploy` (W018, W009, W020) judges the value where it runs and
never `'*'`; Bandit B105/Ruff S105 only a literal `SECRET_KEY = "..."`; pylint/Ruff PLW1508 only a non-`str`
`os.getenv` default (it would flag `os.getenv("DEBUG", True)` as a type error, not as a security issue); flake8-django
has no settings rule. The cross-file helper resolution is the value added.

**Text.** The description (120 characters) passes. The message fails message criteria 6 and 7: the template is 344
characters before `{risk}` (89 to 138 more), and every match gets the same three-setting fix list ("DEBUG false,
ALLOWED_HOSTS without "*", SECRET_KEY with no default ...") whatever setting it is about. The `{fallback}` placeholder
shows the fallback of another variable for CX-E6 ("`DEBUG` falls back to `"development"`"), which reads oddly.

**Verdict: rework**, then `steer`, on. (a) One message per setting, each under 200 characters, e.g. "With DEBUG unset this
falls back to on; make the fallback false and set DEBUG=true only in a development settings module or .env." (b) Add
the fail-open DEBUG shapes CX-E1, CX-E3, CX-E4: a fallback string that is truthy as a str when used without conversion
or through `bool()`, and `!=`/`not in` comparisons of a read without a fallback. (c) Honour a raising check on the
setting in a module that star-imports the base (CX-E2). (d) Fix the README's PLW1508 attribution.

## 2. `django-practice-addfield-default-runs-once`

**Value.** Real and severe. Django evaluates an AddField's default once and writes it into every existing row
(writing-migrations docs; ticket #37031, where a maintainer also warns that one transaction holds the lock). pretix
shows the security form: `0122_orderposition_web_secret` (one ticket-download secret for every order position that
existed), `0078` (one `notifications_token` for every user, used in unsubscribe links), `0147_user_session_token` (one
session token for every user). The sources hold.

**Correctness.** The `null=True` exemption is wrong. Django's schema editor adds a nullable field with its default
too: `BaseDatabaseSchemaEditor.add_field` calls `column_sql(model, field, include_default=True)`, and
`_iter_column_sql` emits `DEFAULT <effective_default>` for a nullable field unless the backend sets
`skip_default_on_alter` (PostgreSQL does not); `_effective_default` calls `field.get_default()`, so
`UUIDField(default=uuid.uuid4, null=True)` gives every existing row one UUID, not NULL (Django 5.2,
`django/db/backends/base/schema.py`, `_iter_column_sql` and `add_field`). The case file pins the miss:
`cases/.../0002_order_tokens.py:133` (`maybe_id`, no backfill) must stay silent. The docs' three-step recipe is still
silent without the exemption, because its RunPython backfill is already recognised (`adr_backfilled`).
Counterexamples (`/tmp/review-django/cx-addfield`):

| Label | Shape | Should | Did |
|---|---|---|---|
| CX-A1 | `AddField("order", "public_id", UUIDField(default=uuid.uuid4, null=True))`, no backfill | report | silent (miss, the exemption above) |
| CX-A2 | `order.token` with a generated default; a later RunPython fills `customer.token` | report | silent (miss: backfill matched by field name, not model) |
| CX-A3 | generated default followed by `RunSQL("UPDATE shop_order SET secret = md5(random()::text …)")` | silent | reported (false positive; README gap) |
| CX-A4 | `default=shortuuid.uuid` | report | silent (miss; README gap) |
| CX-A5 | `SlugField(default="", unique=True)`, `preserve_default=False` | report | reported |

It also reports squashed migrations (`replaces = [...]`), which `alterfield-rewrites-table` and
`runpython-with-schema-change` skip: pretix's two squashes repeat the findings of `0078` and `0122`.

**Corpus.** 8 matches in 3 repositories, all read, 6 real (75%):

- pretix `0078_auto_20171206_1603.py:57`, `0122_orderposition_web_secret.py:18`, `0147_user_session_token.py:18`:
  real (shared notification, ticket and session secrets).
- pretix `0077_…_squashed_0088_….py:203` and `0108_…_squashed_0141_….py:257`: the same two AddFields inside squashes;
  the hazard holds for a database at the squash's start, but each is a duplicate of a finding above. Counted real.
- plane `0099_profile_background_color_profile_goals_and_more.py:19,31`: `get_random_color` gives every existing
  profile and workspace one background colour. True, but cosmetic: counted not real.
- saleor `saleor/account/migrations/0046_user_jwt_token_key.py:19`: `partial(get_random_string, length=12)` gives every
  existing user one `jwt_token_key`, the per-user key that revoking a user's tokens rotates: real.

With (c) and (d) of the verdict applied, the same corpus gives 4 matches, 4 real; (a) can only add matches, which
were not measured here.

**Overlap.** django-migration-linter has no such check; its `NOT_NULL` and `ADD_UNIQUE` checks flag some of the same
migrations for other reasons (a NOT NULL column, a unique index under lock) and say nothing about a shared value.
pylint-django's `new-db-field-with-default` (W5198, opt-in) flags every AddField with any default, for lock time.
`django-practice-default-called-once` covers `default=uuid4()` in models. The rule adds the generator analysis.

**Text.** Description (142) and messages (182, 189) are within limits. Two message problems: (1) "or use db_default"
is unsafe advice on PostgreSQL: a per-row `db_default` is a volatile expression (`RandomUUID()`, `Random()`), and
adding a column with a volatile default rewrites the whole table under ACCESS EXCLUSIVE (PostgreSQL ALTER TABLE notes;
the hazard `pg.volatile-default-rewrites`), so following the advice trades a shared value for a table rewrite on a
large table; and `get_random_string` or `secrets.token_*` have no database equivalent. (2) Every corpus match is in a
migration applied long ago; unlike `runpython-with-schema-change`'s, these messages never say to leave an applied
migration alone and repair the data in a new one.

**Verdict: rework**; `log` until reworked (75% as shipped), then `steer`, on. (a) Drop the `null=True` exemption; keep the backfill exemption. (b) Match a
backfill by model as well as field name. (c) Skip squashed migrations. (d) Leave `random.*` defaults out of
`SharedValue` (or give them their own log-mode message): every corpus hit of it is cosmetic. (e) Message: drop "or use
db_default", or qualify it ("db_default rewrites the table on PostgreSQL"), and add "for a migration already applied,
regenerate the values in a new data migration".

## 3. `django-practice-bulk-ignore-conflicts-used`

**Value.** Real: `bulk_create(..., ignore_conflicts=True)` sets no keys and returns the input list (QuerySet docs;
Django forum thread 38435; ticket #34698 made `update_conflicts` return keys, not `ignore_conflicts`). plane's invite
views mail tokens the database never stored when an invitation already existed (`app/views/workspace/invite.py:120`,
`app/views/project/invite.py:120`): a re-invite sends a link that cannot work. Teams hit it, and the workaround
comments in plane, label-studio and DefectDojo show it.

**Correctness.** The rule treats every element as "not the stored row" whether or not anything can conflict, and
whether or not the code set the key itself. Counterexamples (`/tmp/review-django/cx-bulk`):

| Label | Shape | Should | Did |
|---|---|---|---|
| CX-B1 | `Tag` with `name` as primary key; `bulk_create(tags, ignore_conflicts=True)`; `article.tags.add(*tags)` | silent (the key is the name; a conflict means that row exists) | reported (false positive) |
| CX-B2 | same objects, `Link(tag_id=t.pk)` | silent | reported (false positive) |
| CX-B3 | `Event(id=k, …)` with the key given; `[e.id for e in events]` | silent | reported (false positive) |
| CX-B4 | `for inv, line in zip(invites, lines): line.invite = inv; line.save()` | report | silent (miss: attribute assignment, and `zip`) |
| CX-B5 | `post_save.send(sender=Invite, instance=inv, created=True)` for each object | report | silent (miss) |
| CX-B6 | `return {"created": len(created)}` | report | reported |

CX-B1 to B3 are the safe uses: when the conflict is on a key the code supplies, the object names the stored row
either way. The same holds when nothing but a generated UUID key is unique (no conflict can happen).

**Corpus.** 13 matches in 3 repositories, all read (saleor, pretix, label-studio and the others: 0):

- plane, real (7): `app/views/workspace/invite.py:120`, `app/views/project/invite.py:120` (tokens mailed, not
  stored); `app/views/issue/label.py:115` (`Label` is unique per project and name: the response serializes labels
  whose ids name no row); `app/views/issue/relation.py:307,312`; `api/views/cycle.py:1059`;
  `api/views/module.py:780` (unique link tables; serialized ids of skipped rows).
- plane, not real (4): `bgtasks/dummy_data_task.py:244,334,351,372`. `Page` and `Issue` have no unique constraint
  but the generated UUID key (`db/models/page.py:23`, `db/models/issue.py:104`), so no object can conflict; the
  `ignore_conflicts=True` is dead, and the objects are the stored rows.
- addons-server `src/olympia/users/tasks.py:274`: logs `len(created_objects)` "of" `len(batch)` as if they could
  differ: real, harm limited to a misleading log line.
- DefectDojo `dojo/tags/utils.py:161`: `batch_created = len(actually_created)` under the comment "Count how many were
  actually created", added to the tag's count: real.

Precision 9 of 13 (69%). Without the 4 dead-`ignore_conflicts` matches (verdict (b)), 9 of 9.

**Overlap.** None: Ruff, pylint-django, flake8-django and django-upgrade have no ignore_conflicts check.

**Text.** Description 167, passes. Messages are 343 and 321 characters, over criterion 6's 200. Their "or, if no
object can conflict, drop ignore_conflicts" is the right escape for the dummy-data matches.

**Verdict: rework**; `log` until reworked, then `steer`, on, if its re-measured precision is 90% or more. (a) Exclude
objects whose primary key the code sets (a non-auto primary key field of the model, or `pk=`/`id=`/`<pk name>=` in the
constructor). (b) Exclude models whose only unique constraint is a generated primary key (no conflict possible), or
give them a message that says to drop `ignore_conflicts`. (c) Cut both messages under 200 characters. (d) Optional:
attribute assignment of an element to a foreign key (CX-B4).

## 4. `django-practice-runpython-with-schema-change`

**Value.** Real, two hazards. The pending-trigger failure is documented in the RunPython reference, explained by
Zulip's migration guide, and collected in Stack Overflow 12838111 (score 200). DefectDojo's own migrations show a team
meeting it: `0297_customizable_asset_attributes.py:81-90` runs `SET CONSTRAINTS ALL IMMEDIATE` with a comment naming
"pending trigger events", and `0286_cicd_infrastructure.py:145` does the same between its RunPython and its
alterations. The locked-backfill case is Django ticket #37031's maintainer comment ("This will hold a lock on the table
for the whole duration of the transaction").

**Correctness.** The foreign-key analysis (which writes queue checks, `SET CONSTRAINTS … IMMEDIATE` around a
changed key, keys added earlier in the migration) is careful. Counterexamples (`/tmp/review-django/cx-rpsc`):

| Label | Shape | Should | Did |
|---|---|---|---|
| CX-S1 | AddField, then a RunPython that updates one row by `pk` | silent (lock held for milliseconds) | reported, `LockedBackfill` (false positive) |
| CX-S2 | RunPython deleting duplicate customers with `schema_editor.execute("DELETE …")`, then a unique AlterField | report | silent (miss: raw SQL, a stated gap) |
| CX-S3 | the same delete through the historical model, then AddIndex | report | reported, `PendingTriggerEvents` |
| CX-S4 | `apps.get_model("shop", name)` in a loop, after AddField | report | silent (miss, a stated gap) |

Not tested, from the PostgreSQL source (`RI_FKey_fk_upd_check_required`): an update of rows the same transaction
rewrote (an AlterField that rewrites the table, then a `save()` backfill) queues checks even with unchanged keys,
because the rows' `xmin` is the current transaction; such a migration gets `LockedBackfill` where
`PendingTriggerEvents` is the outcome. A `RunSQL("SET CONSTRAINTS ALL IMMEDIATE")` between the RunPython and the
alteration (DefectDojo `0286`) is not read as a fix; in the corpus it only meets the before-order case.

**Corpus.** The criterion for "real": the lock covers a table that grows with use or tenants (orders, issues, users,
activity, log entries; in multi-tenant plane and pretix, per-project tables too) and the RunPython touches most of
its rows; not real: per-organisation configuration tables (teams, metadata properties, app and mail configuration)
and a filtered handful of rows.

| Repository | Matches | Real | Not real |
|---|--:|--:|---|
| linkding | 1 | 1 | |
| healthchecks | 0 | | |
| paperless-ngx | 1 | 1 | |
| label-studio | 0 | | |
| plane | 18 | 17 | `0063_state_is_triage_alter_state_group.py:43` (filtered update of "Triage" states) |
| InvenTree | 4 | 2 | `part/0145_auto_20251203_1238.py:86` (parameter templates), `users/0017_…:58` (API tokens) |
| pretix | 29 | 22 | `team` ×4 (`0138`, `0184`, `0236`, `0298`), `0250` eventmetaproperty, `pretixdroid/0004` appconfiguration, `sendmail/0004` rule |
| addons-server | 0 | | MySQL, skipped by design |
| DefectDojo | 5 | 4 | `0272_reencrypt_tool_config_credentials_aes_gcm.py:112` (tool configurations) |
| saleor | 27 | 13 | admin-bounded tables: `menu` ×3, `page` ×2, `pagetranslation` ×2, `appextension`, `attributevariant`, `attribute`, `producttype`, `pluginconfiguration`, `warehouse`, and `sitesettings` (one row per site, read through `Site.objects.get_current()`, which `rps_one_row` does not know); real: `order` ×3, `orderline` ×5, `orderevent`, `cartline`, `checkout`, `transactionitem`, `productvariant` |
| **Total** | **85** | **60 (71%)** | |

Real samples: plane `0045_…:138` (every `issueactivity` with `field="priority"` loaded and bulk-updated after its
AddField), `0067_issue_estimate.py:257` (a per-project, per-issue loop with the `issue` table locked), pretix
`0252_logentry_organizer.py:53` (every log entry), InvenTree `audit/0002_actor_username.py:49`, paperless
`documents/0016_sha256_checksums.py:129` (reads every original file with `documents_document` locked), linkding
`bookmarks/0044_bookmark_latest_snapshot.py:44`, DefectDojo `0042_risk_acceptance_improvements.py:27` and
`0291_dojometa_location_product.py:72`. The one `PendingTriggerEvents` match is DefectDojo
`0263_language_type_unique_language.py:91`: it reassigns `Languages` rows, deletes duplicate `Language_Type` rows a
foreign key points at, then makes `Language_Type.language` unique in the same transaction: real, it fails on a
database that has duplicates. DefectDojo `0286_cicd_infrastructure.py:137` (`LockedBackfill` on `engagement`) is real;
its `RunSQL("SET CONSTRAINTS ALL IMMEDIATE")` at line 145 shows the team had met the pending-trigger error there.

A caveat the precision does not show: linkding, paperless and InvenTree run `migrate` at container start with the
application stopped, so a lock there costs upgrade time, not an outage. The hazard is an outage for teams that migrate
under traffic (plane cloud, pretix hosted, saleor cloud).

**Overlap.** None. django-migration-linter checks neither (it reads SQL; a RunPython has none); Squawk reads SQL;
pylint-django has no such check.

**Text.** Description 167, passes. Messages 390 (`PendingTriggerEvents`) and 306 (`LockedBackfill`), over 200. The
`PendingTriggerEvents` message should also offer the fix teams use, `RunSQL("SET CONSTRAINTS ALL IMMEDIATE")` before
the schema change (DefectDojo; SO 12838111's third answer).

**Verdict: rework (text; keep the logic)**; `log`, off (`"on": false`). (a) Both messages under 200 characters. (b) Offer `SET
CONSTRAINTS ALL IMMEDIATE` in the pending-trigger message, and read it as a fix when a RunSQL with it sits between
the RunPython and the alteration. (c) Skip `LockedBackfill` when every write is filtered by primary key (CX-S1).
Precision is 60 of 85 (71%). BRIEF's bar for `"on": true` is 90%, and `pack.json` leaves `on` unset, which means on:
set `"on": false`, as the pack did for `get-handler-writes` (72%) and `host-header-without-get-host` (83%).

## 5. `django-practice-alterfield-rewrites-table`

**Value.** Real: `ALTER COLUMN … TYPE` rewrites the table and its indexes under ACCESS EXCLUSIVE unless the types are
binary coercible (PostgreSQL ALTER TABLE notes; Squawk `changing-column-type`). healthchecks' `ping.id` →
BigAutoField and pretix's `0246_bigint.py` (63 tables in one migration, among them `order`, `orderposition`,
`logentry`) are exactly the migrations that take a large site down for the rewrite.

**Correctness.** The type classification is right on every counterexample and on every corpus match read: each
reported change does rewrite; widening varchar and varchar → text stay silent. Counterexamples
(`/tmp/review-django/cx-alterfield`):

| Label | Shape | Should | Did |
|---|---|---|---|
| CX-L1 | IntegerField → BigIntegerField | report | reported |
| CX-L2a | `customer.id` AutoField → BigAutoField | report | reported |
| CX-L2b | `order.customer_ref` IntegerField → ForeignKey to that bigint key | report (int → bigint) | silent (miss; stated gap) |
| CX-L3 | IntegerField → PositiveIntegerField (adds a CHECK: a full validating scan under ACCESS EXCLUSIVE) | out of scope | silent |
| CX-L4 | DecimalField(10,2) → (12,3) | report | reported |
| CX-L5 | CharField(20) → CharField(40) | silent | silent |
| CX-L6 | RenameField, then a shorter CharField | report | silent (miss; stated gap) |
| CX-L7 | a shorter CharField on a table created one migration earlier | silent if the table is new | reported (cannot know) |

Two shapes the rule could settle and does not: a model read as one row (paperless's `ApplicationConfiguration`,
which `runpython-with-schema-change` already exempts through `dj_one_row`), and a run that configures only MySQL
(addons-server), which `runpython-with-schema-change` detects and this rule does not; there the message's
"PostgreSQL rewrites …" is about the wrong database, though its MySQL parenthetical holds.

**Corpus.** Criterion: real when the table's row count grows with use or tenants; not real for a table bounded by
configuration.

| Repository | Matches | Real | Not real |
|---|--:|--:|---|
| linkding | 0 | | |
| healthchecks | 1 | 1 | |
| paperless-ngx | 23 | 1 | 22: correspondents, document types, tags, storage paths, saved-view rules, workflow triggers and actions, mail accounts and rules, and 4 on the one-row `applicationconfiguration` (`documents/0010_…`, `paperless/0007_…`, `paperless_mail/0002_…` "optimize_integer_field_sizes"); real: `workflowrun.type` |
| label-studio | 0 | | |
| plane | 9 | 9 | multi-tenant: `state`, `issuetype`, `cycle` grow with projects |
| InvenTree | 4 | 4 | `notificationmessage` ×2, `notificationentry`, `stockitem.serial` |
| pretix | 82 | 74 | `globalsettingsobject_settingsstore`, `staffsession`, `staffsessionauditlog`, `exchangerate`, `registeredapplepaydomain`, `u2fdevice`, `webauthndevice`, `mediumkeyset` |
| addons-server | 7 | 3 | real: `rating.ip_address`, `rating.deleted`, `userprofile.biography`; not: `shelf` ×3, `cinderpolicy` |
| DefectDojo | 7 | 6 | `0024_cve_fix_1553.py:22` (`finding_template`, admin templates); real: `finding.cve`, `finding.title`, `finding.hash_code`, `finding.publish_date`, `endpoint_status.date`, `alerts.id` |
| saleor | 20 | 14 | `sale` ×3 (`discount/0013_…:14,19`, `0021_…:15`), `collection.slug`, `collectionchannellisting.publication_date`, `warehouse.name`; real: `order` ×2, `orderline` ×3, `transaction.amount`, `voucher` ×3, `promotionrule_variants.id`, `product`, `productvariant`, `productchannellisting` ×2 |
| **Total** | **153** | **112 (73%)** | |

78 of pretix's 82 are one-line `id` AlterFields in `*_bigint.py` files and were judged by table; the other matches
were read in their files. Real samples: healthchecks `hc/api/migrations/0037_auto_20180127_1215.py:13` (`ping.id`,
the largest table), InvenTree `stock/migrations/0050_auto_20200821_1403.py:16` (`stockitem.serial` integer →
varchar), plane `0084_…:51` (`inboxissue.source` text → varchar(255)), addons-server
`users/migrations/0009_alter_userprofile_biography.py:26`. False-positive samples: paperless
`paperless/migrations/0007_optimize_integer_field_sizes.py:35,44,53,62` (one-row configuration), addons-server
`shelves/migrations/0016_auto_20210617_1113.py:25,30`.

One migration can carry dozens of findings (pretix `0246_bigint.py`: 63; paperless `documents/0010`: 9): correct,
but a reader sees one decision 63 times.

**Overlap.** django-migration-linter `ALTER_COLUMN` flags every `ALTER COLUMN … TYPE` in the SQL as an error, the
catalog-only ones included, with "Could be backward compatible. You may ignore this migration." Squawk's
`changing-column-type` flags every type change, on SQL (`sqlmigrate` output). The rule adds the classification from
the migration history, so what it reports really rewrites; it is the only one of the three that reads Django
migrations without a database.

**Text.** Description 169, passes. The message is 336 characters, over 200; and on a MySQL-only project it leads with
PostgreSQL.

**Verdict: rework**; `log`, off (`"on": false`). (a) Message under 200 characters. (b) Reuse `dj_one_row` (plus `.first()`, as
`runpython-with-schema-change` does) to skip one-row tables. (c) Skip, or word for MySQL, a run that configures only
MySQL. Precision is 112 of 153 (73%), below the 90% bar for `"on": true`, and `pack.json` leaves `on` unset (on): set
`"on": false`. Table size is not visible to it, so `log` stays after the rework.

## 6. `django-practice-runpython-live-model`

**Value.** Real but narrower than the others: the live model breaks a migration only on a database that runs it after
the model has changed (a fresh install with data, CI from an empty database when the query fails on missing columns,
a self-hosted upgrade across several releases). For self-hosted products that is every upgrade; label-studio's
`tasks/functions.py:131-132` comment ("some columns … aren't created until after a migration calls this code") is a
team that was bitten. Source: Django's "Historical models" ("may work initially but will fail in the future").

**Correctness.** Counterexamples (`/tmp/review-django/cx-live`):

| Label | Shape | Should | Did |
|---|---|---|---|
| CX-R1 | `recount(apps)`: helper uses `apps.get_model` when given `apps`, the live class only when `apps is None` (through a local `model = Book`) | silent | silent |
| CX-R2 | `rerank(apps)`: helper falls back to the global registry, then `apps.get_model` | silent | silent |
| CX-R3 | `touch_all(apps)`: helper takes `apps` and still uses `Book.objects` | report | reported |
| CX-R4 | `stats(apps)`: `if apps is None: return Book.objects.count()` | silent | reported (false positive: the None guard is read only for a keyword argument) |
| CX-R5 | `call_command("reindex")`, whose command iterates `Book.objects` | report | silent (miss; stated gap) |

**Corpus.** 8 matches in 3 repositories, all read (the other seven: 0), all real in the rule's sense (the code runs today's
model inside an old migration):

- label-studio `data_manager/migrations/0008_manual_counters_update.py:27` and
  `tasks/migrations/0024_manual_migrate_counters_again.py:27` (`calculate_stats_all_orgs` → `Organization.objects`,
  then `Project.objects…values(...)`); `tasks/migrations/0030_auto_20221102_1118.py:8` (`fill_annotations_project`);
  `0042_auto_20230810_2304.py:7` and `0043_auto_20230825.py:7` (`fill_predictions_project`, whose
  `AsyncMigrationStatus.objects.create(...)` inserts every column today's model has).
- InvenTree `common/migrations/0043_auto_20260518_1206.py:47` (`common.tasks.rebuild_attachment` loads and saves the
  live `Attachment` while its docstring claims to mirror the logic "at the time of writing"); `0041_auto_20251203_1244.py:18`
  (`convert_physical_value` → `CustomUnit.objects.all()` inside `try/except Exception`; on PostgreSQL a failed query
  aborts the migration's transaction, so the `except` does not save it).
- DefectDojo `dojo/db_migrations/0105_endpoint_host_migration.py:7`: `clean_hosts_run(apps=apps, …)` uses the
  historical models it is given, but calls `endpoint_filter` (`dojo/endpoint/utils.py:28-29`), which queries the live
  `Endpoint.objects` and iterates the rows (`[x.id for x in ep]`). Real, and the README's "left alone" list names
  this very helper as silent: the README is out of date, the rule is right.

Three of the label-studio queries (`values_list('id')`, `update(project_id=…)`) name only columns that exist at that
migration, so they work today; the finding is latent. The `create(...)` and full-row loads can fail now.

**Overlap.** django-migration-linter `RUNPYTHON_MODEL_IMPORT` regex-scans the RunPython function's own source for
`X.objects` without a `get_model` assignment; `discover_function` returns the RunPython `code`, so it also covers
`RunPython(helper)` when the helper itself uses `X.objects` (this rule's `rpl_code` exit). What the rule adds is the
helper reached through a call, one or more modules away, which is every corpus match.

**Text.** Description 151 and message 195 pass. The message is accurate and gives a safe change.

**Verdict: keep**, `steer`, on. Optional: read a None guard passed by position (CX-R4).

## Pack-level findings

**Missing rules, with evidence.**

- The fail-open DEBUG shapes (CX-E1, E3, E4) belong in `settings-env-fails-open`, not in a separate rule; see its
  verdict.
- A volatile default added to an existing table (`db_default=RandomUUID()`/`Random()`, or a `RunSQL` `ADD COLUMN …
  DEFAULT gen_random_uuid()`) rewrites the table (`pg.volatile-default-rewrites`). No linter in the list checks
  `db_default`, and `addfield-default-runs-once`'s message currently recommends it. The corpus uses `db_default` in
  label-studio, addons-server, DefectDojo and saleor among others, every time with a constant (`False`, `True`, `0`,
  `None`, `{}`): no evidence of the volatile form yet, so a candidate to keep, not to build now.
- Checked and not recommended: `AddConstraint(CheckConstraint)` on an existing table (validated under ACCESS
  EXCLUSIVE; 11 in the corpus: addons-server 8, saleor 3) and `AddField(ForeignKey)` on an existing table (353; its
  index is django-migration-linter's `CREATE_INDEX`, and the new column is all NULL, so the constraint check is
  short). Squawk's `constraint-missing-not-valid` covers the SQL form.

**Rules in the wrong pack.** None. All six are Django-specific (migrations, the Django ORM, Django settings).
`bulk-ignore-conflicts-used` would have a SQLAlchemy sibling (`on_conflict_do_nothing` with `returning`), which
belongs to the sqlalchemy pack.

**The library.** `rules/lib/django.rerail`'s migrations section (operations, fields by name, histories through
`dj_mig_before`, `dj_runpython`, `dj_one_row`) is good shared vocabulary. Three pieces are duplicated between rules
and should move into it: the field history (`ar_def`/`ar_prev`/`ar_near` in `alterfield-rewrites-table` and
`rps_fdef`/`rps_prev`/`rps_old` in `runpython-with-schema-change`, each with its own squash-dependency fix,
`ar_dep` and `rps_mig_depends`); the one-row test (`rps_one_row` extends `dj_one_row` with `.first()`, which
`addfield-default-runs-once` lacks); and the MySQL-only test (`rps_mysql_only`), which `alterfield-rewrites-table`
needs. The squash rule differs between rules (two skip `replaces` migrations, `addfield-default-runs-once` does not).

**README.** Four things in "Rule details" are wrong or missing. The sections of `alterfield-rewrites-table` and
`runpython-live-model` still hold the literal placeholder `CORPUS_TABLE` (README.md lines 1317 and 1467): their corpus
numbers were never written. `runpython-live-model`'s "left alone" list names DefectDojo's `clean_hosts_run(apps, ...)`
as silent, but the rule reports it, rightly (section 6). `bulk-ignore-conflicts-used` counts plane's four
`dummy_data_task.py` matches as real; nothing in them can conflict. `settings-env-fails-open`'s "Gaps" credits Ruff's
PLW1508 with the string-truthy DEBUG, which it does not flag.

**Pitching the pack on its own.** The settings rule and the corrected addfield rule are the pitch: each finds a
security bug in well-known projects that no linter in a Django team's toolchain reports (fail-open settings in
healthchecks, label-studio, plane, addons-server and saleor; shared session, ticket and JWT keys in pretix and
saleor). The two lock rules are useful in `log` for a team that migrates under traffic, and noise for a self-hosted
product that migrates with the app stopped; the README should say so. As shipped, the messages of four of the six
rules exceed the 200-character limit, and `bulk-ignore-conflicts-used` would steer on dead `ignore_conflicts` flags:
fix those before pitching.

## Corpus repositories and commits

Shallow clones under `~/rerail-bench/python-arch/corpus/`, commits from `MANIFEST.tsv`. No repository was added.

| Repository | Commit | Python files | Run |
|---|---|--:|---|
| sissbruecker/linkding | `27b7303baf41bb28babc610ac8eaa486e1ddfab5` | 229 | `review-django-six-sissbruecker__linkding` |
| healthchecks/healthchecks | `c1039ad23395162c5e2f17d324d8ccdaa60a1d14` | 658 | `review-django-six-healthchecks__healthchecks` |
| paperless-ngx/paperless-ngx | `faa6f09f289bad52fe8dce9d97ab9e121ae37e6b` | 424 | `review-django-six-paperless-ngx__paperless-ngx` |
| HumanSignal/label-studio | `2f42caecbf6c237d42463626860308d7262afc9f` | 775 | `review-django-six-HumanSignal__label-studio` |
| makeplane/plane | `723bb178f97b87e80a23df9eed134eb1388d7465` | 784 | `review-django-six-makeplane__plane` |
| inventree/InvenTree | `939df605f43db4998bf67fb7f5d5d22d65762e9e` | 731 | `review-django-six-inventree__InvenTree` |
| pretix/pretix | `40ea9097d0b290b4fb9c42fc61c62f86c9adc892` | 1,228 | `review-django-six-pretix__pretix` |
| mozilla/addons-server | `7e3374fe5383c5e90262155245917f100b8fac30` | 1,426 | `review-django-six-mozilla__addons-server` |
| DefectDojo/django-DefectDojo | `cd0a4483a224707f72a0fc3112359089c1446b55` | 2,601 | `review-django-six-DefectDojo__django-DefectDojo` |
| saleor/saleor | `e1ccf4d580025b06c2ff01441a1e798dc4635f24` | 4,385 | `review-django-six-saleor__saleor` |

## Applied

Applied after the review, by root cause rather than by the counterexamples alone. Corpus numbers re-measured on the
same 10 repositories (runs `django-fix-*`); `pack.json` and the README list the rules in the reading order above.

| Rule | Mode / on | Corpus after (review) |
|---|---|---|
| `settings-env-fails-open` | steer / on | 6 of 6 real (6 of 6) |
| `addfield-default-runs-once` | steer / on | 6 of 6 real; 4 of 6 if pretix's two unused `invite_token`s count as harmless (6 of 8) |
| `bulk-ignore-conflicts-used` | steer / on | 8 of 8 real on the 10 repositories, 10 of 10 with Tandoor (9 of 13) |
| `runpython-with-schema-change` | log / off | 65 of 89 real, 73% (60 of 85) |
| `alterfield-rewrites-table` | log / off | 129 of 172 real, 75% (112 of 153) |
| `runpython-live-model` | steer / on | 8 of 8 real (8 of 8) |

- **settings-env-fails-open.** (a) Three violations, one per setting (197, 186, 192 characters), DEBUG's without a
  `{fallback}`. (b) DEBUG's value is evaluated as Python would with the variable unset, instead of matched by shape:
  raw fallbacks, `None` without one, `bool`/`int`/`str`, decouple `cast=`, string methods, `==`/`!=`/`in`/`not in`/
  `is None`/`not`/`and`/`or`; CX-E1, E3, E4 and their variants are caught. (c) A refusal of the default counts in the
  module that assigns the setting or in every non-development module that star-imports it (CX-E2 silent); relative
  star imports are resolved from the importing file, not matched by path suffix, which had let one project's
  `production.py` stand for every `*/settings/base.py`. (d) README Gaps no longer credit PLW1508.
- **addfield-default-runs-once.** (a) `null=True` exemption removed. (b) A backfill counts only for the model it names
  (`apps.get_model` in any form, or a manager of a name bound to the model) or a RunSQL `UPDATE` of the model's
  table (CX-A2 caught, CX-A3 silent). (c) Squashes skipped. (d) The `random` module left out of the generators;
  `random.SystemRandom` and `shortuuid` added (CX-A4). (e) `db_default` advice dropped; both messages say to
  regenerate the values in a new data migration when the migration is already applied.
- **bulk-ignore-conflicts-used.** The model inserted is resolved (imports, aliases, re-exports, any manager name,
  `self.model.objects`, the objects' constructor) with its key and unique constraints across bases in other files.
  (a) Objects whose key the code gives are left alone (CX-B1, B2, B3). (b) A model no row can conflict on (only a
  generated key unique) is no match (plane's dummy data). (c) Messages 199 and 194 characters. (d) Elements are
  followed through copies, slices, `zip` and `enumerate`, into foreign-key attributes and any signal's
  `send(sender=…)` (CX-B4, B5 caught).
- **runpython-with-schema-change.** (a) Messages 199 and 196 characters, without the `atomic = False` advice. (b)
  The pending-trigger message offers `SET CONSTRAINTS ALL IMMEDIATE`, and a RunSQL with it before the alteration (or
  the statement in the RunPython's code) is read as the fix. (c) `LockedBackfill` skips a RunPython whose every
  write touches one row once: a `get()`/`first()`/`last()` row, a filter on a unique column (the key or a field the
  migrations make unique), a single insert, with no loop around the write or the call into its function (CX-S1
  silent; the same write in a loop is reported). A model named by a loop over literal names is resolved (CX-S4).
  Raw SQL (CX-S2) stays a gap.
- **alterfield-rewrites-table.** (a) Message 180 characters. (b) One-row tables skipped through the library's
  `dj_one_row`. (c) A MySQL-only run skipped (`dj_mysql_only`). Types are resolved, not named: project field
  classes through their bases (`dj_custom_field`; length and precision only when no `__init__` on the way can
  rewrite them), a relation's column by its target key's type at that point of the history (CX-L2b caught), and the
  history follows a `RenameField` (CX-L6 caught). CX-L7 (a table created one migration earlier) stays a gap.
- **runpython-live-model.** The optional CX-R4: a None guard is read when the caller passes the parameter by
  position too (after `self`/`cls` for a method). Found on the way: the helper walk was keyed by helper, not by the
  call out of the migration, so one call's arguments could block or unblock another's; it is now kept per call.
  The README's "left alone" claim about DefectDojo's `clean_hosts_run` is corrected.
- **Library** (`rules/lib/django.rerail`, used only by these six rules). The field history (`dj_field_old`,
  `dj_field_at`, `dj_mig_ran_before`) moved there from both lock rules; it now follows `RenameField`, reads a
  dependency on a squashed-away migration as the squash, and never puts a squash before a migration it replaces
  (that had hidden pretix's `0120_…` change). The one-row test (`dj_one_row`) no longer takes a bare `first()`, which
  reads one row of many (saleor's `.semgrep` example had made `product` a one-row table); it takes the lazy
  singleton (`first()` beside a `create()`) and the current site's row (`Site.objects.get_current().<related name>`).
  `dj_mysql_only`, `dj_custom_field`, `dj_custom_field_init`, `dj_pk_call` and `dj_fk_target` are new.
- **README.** No `CORPUS_TABLE` left; the six rules sit under Rule details in reading order with their own table;
  bulk-ignore-conflicts-used no longer counts plane's dummy data as real; the lock rules say they matter for a team
  that migrates under traffic.

Not done: the missing rules this review lists (a volatile `db_default` or `RunSQL` default on an existing table,
`pg.volatile-default-rewrites`; the SQLAlchemy sibling of bulk-ignore-conflicts-used) are out of scope. The bulk
rule's model and Meta helpers (`bic_*`) stay inlined: no other rule of the six uses them.
