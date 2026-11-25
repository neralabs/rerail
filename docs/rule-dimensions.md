# Rule dimensions

A rule pack is one stack: a language (`python`), a framework (`django`, `react`) or a repository (`rerail`). A team's
code spans several stacks, and the questions it asks about its rules cut across them: which rules keep our data safe,
which guard against security holes, which ones are about speed. A rule's dimensions answer those questions. They are
given per rule in `pack.json` ([pack format](pack-format.md#packjson)) and read by the dashboard and the rule packs API.

```json
{
  "id": "django-practice-enqueue-on-commit",
  "document": "rules/django-practice-enqueue-on-commit.rerail",
  "title": "Django practice: a task enqueued or a mail sent inside `transaction.atomic`",
  "group": "Django practice",
  "dimensions": { "risk": "data-safety", "layer": "jobs", "severity": "critical", "hazard": "tx.side-effect-before-commit" }
}
```

- [The dimensions](#the-dimensions)
- [Hazard ids](#hazard-ids)
- [Where they show](#where-they-show)
- [Side packs](#side-packs)

## The dimensions

`risk`, `layer` and `severity` are required together; `hazard` is optional. The values are fixed
(`RULE_RISKS` and `ruleDimensionsSchema` in `src/engine/catalog/rule-packs.ts`); a pack that uses another is refused.

**`risk`**: what goes wrong when the rule is broken. One per rule, the worst it guards against.

| Value | What goes wrong |
|---|---|
| `security` | Someone can read, change or run what they should not: injection, missing auth, secrets, exposed fields, XSS, SSRF, one user's data cached for another |
| `data-safety` | Stored data is lost, corrupted, duplicated or left inconsistent: transactions, bulk writes, lost updates, destructive migrations, work dispatched before commit |
| `availability` | The app goes down, blocks or fails to deploy: locking DDL, blocking calls in async code, connection or resource exhaustion, crashes at runtime, migrations that fail |
| `correctness` | A silently wrong result or broken behaviour, with no data lost: stale cache, wrong key, swallowed redirect, missing return |
| `performance` | Needless work or cost: N+1 queries, re-renders, clients built per request, unbounded reads |
| `maintainability` | Harder to change safely: deprecated APIs, code in the wrong layer or owner, duplication |
| `test-quality` | Tests that pass without checking what they claim, or that interfere with each other |

The risks follow the categories of change-requesting code review found across open-source projects: security and
privacy; data and persistence; concurrency, async and error handling; correctness; performance; architecture, reuse,
API design and naming; testing.

**`layer`**: where the code the rule reads lives.

| Value | Code |
|---|---|
| `ui` | Components, hooks, client state |
| `server` | Routes, views, handlers, server components and actions, middleware, dependencies |
| `data-access` | ORM queries, sessions, clients, transactions |
| `jobs` | Task queues and background work |
| `migrations` | Schema migrations and DDL |
| `tests` | Test code |
| `config` | Settings, environment, build, deploy, packaging |
| `code` | Language-level code, no framework |

**`severity`**: the worst realistic outcome of one violation that ships. `critical` (a breach, data loss or an
outage), `high` (a failure a user sees), `medium` (wrong or slow in some cases), `low` (friction).

## Hazard ids

A hazard id names one hazard whatever the stack: `<area>.<hazard>`, each part lowercase words joined by dashes. Rules
in different packs that catch the same thing share it, so the hazard can be followed across stacks: an index built
without `CONCURRENTLY` is the same hazard in a Prisma `migration.sql`, an Alembic `op.create_index` and a Django
`AddIndex`. Reuse an id before adding one.

| Area | Ids |
|---|---|
| `pg`: PostgreSQL migrations | `pg.index-blocks-writes`, `pg.concurrently-in-transaction`, `pg.not-null-without-default`, `pg.set-not-null-scans`, `pg.constraint-without-not-valid`, `pg.type-change-rewrites`, `pg.volatile-default-rewrites`, `pg.no-lock-timeout`, `pg.column-dropped-while-used`, `pg.rename-as-drop-add`, `pg.irreversible-migration` |
| `tx`: transactions | `tx.side-effect-before-commit` (a task, mail or HTTP call inside a transaction), `tx.outer-handle-in-transaction`, `tx.writes-without-transaction`, `tx.lock-outside-transaction`, `tx.read-modify-write` |
| `orm`: data access | `orm.query-in-loop`, `orm.bulk-write-skips-hooks`, `orm.unfiltered-bulk-write`, `orm.raw-sql-interpolation` |
| `async` | `async.blocking-call-in-async` |
| Web: `auth`, `input`, `data` | `auth.mutation-without-auth`, `input.mass-assignment`, `data.sensitive-fields-exposed` |

## Where they show

- **Dashboard.** A pack's page offers each risk its rules have under Risk in the Filter, with how many rules have it,
  and each row shows the rule's risk and severity; its layer and hazard show on hover. Search finds a rule by its risk
  (its name or its value: `data safety` or `data-safety`), severity, layer or hazard.
- **API.** `GET /api/rule-packs` (`rule_packs_list`) returns each rule's `dimensions` where its pack gives them.
- **Packs.** Rerail's own rules give them, starting with the `django` and `rerail` packs. The React pack's practice
  rules wait for its next move to a newer Rerail line: giving them now would make the pack need Rerail 0.2.2. A port
  may give them too; its upstream's own categories (Clippy's groups, Ruff's prefixes, ESLint's rule types) are too
  coarse or disagree on what they sort by, so ports are tagged by hand when they are.

## Side packs

Not built yet. This section is the design the dimensions lead to.

A side pack is a selection across the stack packs a repository has, by dimension: **Data safety**, **Security**,
**Zero-downtime Postgres** (`availability` in `migrations`). It holds no rules. It is a query over the dimensions
(`{ "select": { "risk": ["data-safety"] } }`, or `{ "risk": ["availability"], "layer": ["migrations"] }`) with a
starting `on` and `mode` for what it selects. Adding one to a folder turns on, in the mode it gives, every rule it
selects from the packs attached there; a rule a pack adds later joins it. Rules stay in their one pack.

What building it takes:

- A pack kind for it in `pack.json`: `select` in place of `rules`, and the start it gives.
- One more level in [precedence](rule-scoping.md#precedence): a rule's own setting, then a side pack's, then its pack's
  start. `repository_rules_get` names the side pack as what decided a field.
- Set-up offers side packs after the stack packs it matched, with the rules each would turn on in this repository.
- Enough tagged rules for each side pack to be worth adding. Today: Data safety has 8 Django rules, Security 2 Django
  and 2 `rerail`, Zero-downtime Postgres none. The full-stack Python and TypeScript packs in progress add most of
  them, the migration rules among them.
