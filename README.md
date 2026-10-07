# Rerail

**RERAIL — Team Event Monitor for Proximity and Overlap**

The install instructions in this file and in `docs/installation.md` track `main`; read them from `main` when the checkout is on a feature branch.

Rerail is a local semantic monitor for AI-augmented engineering teams. It inspects live human, model, and tool activity outside the generative model, detects when that activity is semantically close to shared engineering context, and surfaces the relevant guidance at the moment it matters.

- **Team** — humans, agents, and tools working in a shared engineering environment.
- **Event** — user input, model output, tool calls, results, handoffs, and task transitions.
- **Monitor** — runs continuously outside the generative model.
- **Proximity** — detects semantic relevance to decisions, constraints, requirements, and prior work.
- **Overlap** — identifies intersecting work, ownership, approaches, and risks.

Rerail does not depend on an agent remembering to search for context or call a coordination tool. Host adapters observe the interaction stream, a bundled local model evaluates configured semantic rules, and deterministic runtime policies decide when and how to deliver a match.

## The problem

AI increases the amount of engineering work a team can run in parallel. It also increases coordination failures:

- An agent crosses an architectural seam documented in a decision it never saw.
- Two engineers independently solve the same problem in incompatible ways.
- A known failed approach is repeated in another session.
- A requirement disappears between planning, implementation, and integration.
- Repository guidance arrives during review instead of while the change is being made.

Putting more text in every prompt does not solve this reliably. It consumes context, becomes stale, and still asks the generative model to recognize when a particular constraint applies.

Rerail moves that responsibility out of the model loop.

```text
Human input ──────────┐
Model output ─────────┼──▶ normalized events ──▶ local semantic monitor
Tool input and output ┘                              │
                                                    ▼
                                      relevant rule + match evidence
                                                    │
                                                    ▼
                                         log, steer, or block
```

The semantic layer answers:

> Which shared context is relevant to this activity?

It does not answer:

> Is this work correct, authorized, or safe?

Hard policy must remain deterministic. Semantic proximity is an attention signal, not proof of compliance or conflict.

## Vision

Rerail's destination is a live coordination layer for multiplayer engineering environments: every participant receives the relevant team decision, constraint, ownership signal, or prior evidence when their current work intersects it.

The central primitive is a stateful semantic monitor over team events:

```text
current human, agent, or tool event
                    ×
active team assertions and ongoing work
                    ↓
evidence-backed coordination finding
```

A team assertion might describe:

- An architectural invariant.
- A technical decision.
- Active work and its owner.
- A product requirement.
- A known failed approach.
- A required integration contract.

This enables several coordination workflows.

### Just-in-time engineering constraints

When work approaches authentication, migrations, public interfaces, persisted events, or another sensitive concept, Rerail surfaces the applicable repository guidance before the implementation diverges.

### Parallel-work overlap

Compare live activity with active assignments and recent work to identify likely duplication or incompatible approaches early. A semantic match should prompt coordination; it should never independently block or reassign work.

### Architectural alignment

Detect when an event is relevant to an architectural decision, then combine that relevance signal with deterministic evidence such as imports, paths, dependency direction, or tool targets.

### Engineer and agent handoffs

Preserve requirements, decisions, failed attempts, evidence, and open risks so the next participant receives the context relevant to the work they continue.

### Integration readiness

Relate parallel implementation results to shared requirements and integration contracts, highlighting weak coverage for human review without claiming that similarity proves correctness.

## What exists today

Rerail currently ships the local inspection kernel and host adapters:

- Native Model2Vec embeddings and cosine matching.
- Resident compiled rule references.
- User-input, streamed assistant-text, and tool-event inspection.
- Sentence, message, turn, and tool-call boundaries.
- Stateful repeat policies with session replay.
- One rule mode on every rule: `log` records, `steer` delivers guidance, and `block` steers until block enforcement lands.
- OMP and Pi extensions.
- Transport-neutral Claude Code, Codex, and Cursor lifecycle adapters.
- A persistent `RERAIL-EVENTS/1` stream for out-of-process harnesses.
- Generic `mark`, `select`, `project`, and `relate` primitives for custom semantic workflows.
- A bundled, pinned INT8 model that makes no network requests at runtime.

Repository-level semantic rules are usable now. Shared team assertions, cross-session synchronization, ownership routing, acknowledgements, and coordination actions remain product direction rather than current behavior.

## How rules work

A conversation rule is a standing semantic query over one event source and boundary. The reference describes the concept to detect; the action contains the context to deliver.

The Rerail database (`$RERAIL_HOME/rerail.sqlite`) is the source of truth for rules. OMP and Pi load every enabled `conversation` rule effective for the session's repository at session start and re-read them before every ingested event, so a conversation rule toggled or edited in the database applies to the next sample; every runtime compiles the enabled `file` rows into the kernel and re-reads them before each audit, so a file row toggled, edited, or deleted through the daemon, the UI, or plain SQL is enforced (or not) on the next tool call; the Rust kernel only loads and evaluates what the database hands it, and nothing derived is stored. JSON files remain a secondary source for conversation rules: OMP loads `.omp/conversation-rules.json`, Pi loads `.pi/conversation-rules.json`, Claude Code loads `.claude/conversation-rules.json`, Codex loads `.codex/conversation-rules.json`, and Cursor loads `.cursor/conversation-rules.json`. Each path is resolved from the active session's working directory and ignored when absent, and a database row replaces a file rule with the same id. Live refresh when a row changes mid-session is deferred; a session picks up edits on its next start or reload.

The package bundles a starting rule set (`policies/executable-policy-dsl/rules/*.json`). Opt-in packs live one folder per pack under `rules/opt-in/<pack>/*.json` (the TSLint ports in `rules/opt-in/tslint/`) and are seeded disabled. Bootstrap — run by every runtime start and `rerail daemon start` — seeds those documents into the catalog as project rules without clobbering yours: a new id is inserted with its `source_digest`; a row whose content still equals the document it was seeded from follows the bundled document when a package update changes it; a row you edited or created is never touched, and `enabled` is never reset. `rerail rules export` dumps the global and current-project rules to `$RERAIL_HOME/rules.json` (or `--output <file>`) and `rerail rules import <file>` loads one back. `rerail update` first copies the whole database to `$RERAIL_HOME/rerail.sqlite.pre-update`; `rerail reset` never touches rules. Dumps written before the Rerail rule contract are rejected by `rerail rules import`, not converted. A `cosine-rules.json` file is no longer read; rename it to `conversation-rules.json`. A `conversation-rules.json` written before rule modes is skipped with a notice. The rules already in the database are rewritten once by the schema v14 upgrade (rule modes) and once more by the schema v15 upgrade (the Rerail rule contract).

Rule packs (`policies/rule-packs/<id>.json`) group rules that already ship: **TSLint**, the 74 golden TSLint ports, and **nera-landing**, the 92 rules for this repository. A pack lists rules by id with a title and a group, and a match definition (file globs or `package.json` dependencies) used to explain why it fits; it adds no rule documents. `recommendRulePacks` matches a repository's files and manifests against every pack locally, with no model and no network, and returns each fitting pack with its rule count and the reason. Packs are not attached to repositories yet: choosing them and seeding their rules in place of the bundled rules waits on NER-1745. See [Rule packs](docs/rule-packs.md).

```json
{
  "version": 1,
  "rules": [
    {
      "id": "persisted-session-events",
      "mode": "steer",
      "reference": "Changing persisted session events, replay, or branch restoration",
      "match": {
        "operator": "gte",
        "threshold": 0.82
      },
      "sample": {
        "source": "assistant-text",
        "boundary": "sentence",
        "minimumTokens": 3
      },
      "action": {
        "content": "Preserve replay idempotency and verify restoration from an interrupted turn."
      },
      "repeat": {
        "mode": "after-gap",
        "gapTurns": 3
      }
    }
  ]
}
```

`mode` is required on every rule, conversation and file alike:

- `log` records a match for review and never sends guidance.
- `steer` sends the rule's guidance to the agent.
- `block` is stored and reported as `block`; until block enforcement lands, it is delivered exactly like `steer`.

`match.operator` is:

- `gte` for semantic similarity.
- `lte` for semantic dissimilarity.

Thresholds are inclusive and range from `-1` through `1`.

Event sources:

- `assistant-text`
- `user-input`
- `tool:<tool-name>`

Inspection boundaries:

- `sentence`
- `message`
- `turn`
- `tool-call`

Repeat policies:

- `once`
- `always`
- `{ "mode": "after-gap", "gapTurns": 3 }`

Start with a small rule set derived from real incidents, repeated review feedback, and consequential architectural decisions. Rules should deliver a specific action or invariant, not generic advice.

Good:

> This touches persisted session events. Preserve replay idempotency and verify restoration from an interrupted turn.

Weak:

> Remember to consider edge cases and write tests.

## Rerail database

Rerail keeps one SQLite file per Rerail home: `$RERAIL_HOME/rerail.sqlite` (`~/.rerail/rerail.sqlite` when `RERAIL_HOME` is unset). It holds the rule catalog and the execution history of every repository on the machine; every harness adapter, runtime, and the daemon open it. The schema is a set of [Drizzle](https://orm.drizzle.team) tables in `src/schema.ts`; `npm run generate:schema` renders their DDL into `src/schema-sql.ts` (`RERAIL_SCHEMA_STATEMENTS`, committed, checked for drift by `test/rule-store.test.ts`), which `openRerailDatabase` applies to a fresh file. Rerail's database code runs only in Node processes and uses `drizzle-orm/better-sqlite3`. The OMP extension remains inside OMP's external Bun-based host; it never opens SQLite and reaches the database only through the daemon's native-session IPC.

- Rule catalog — `catalog_rules`, `rule_projects`, `rule_repositories`, `rule_project_repositories`, and their bookkeeping tables; see [Rule scoping](docs/rule-scoping.md).
- `sessions` — one row per harness session (`host`, `cwd`, `repository_id`, `started_at`, `ended_at`). `repository_id` is the stable identity of the checkout's repository (shared by its linked worktrees), and it is what attributes history to a repository.
- `events` — the append-only transcript for a session (`kind`, `observed_at`, `payload_json`, and a `payload_v8` blob when the payload is not JSON-representable).
- `work_links` — loose associations from a session to external references (`kind` such as `git-branch`, `github-pr`, `linear-issue`; `reference`; optional `url`). No foreign key on purpose, so links can be recorded by any process before or after the session row exists.
- `audits` — every run of rules over a session, live or re-run, with independent scope (`repository`, `touched-files`, `ai-output`) and run mode (`steer`, `log`), selected paths, session, source revision, timestamps, violations, and concrete execution failures. Terminal records are immutable; closing or reopening a session interrupts its unfinished audits without discarding partial results.

The daemon's listings — sessions, audits, violations, repository audits, and rule activity — cover only the repository it was started in; a session addressed by its id is found whichever repository recorded it. File-rule path filters (`scope.include`, `scope.exclude`, exclusions winning) narrow a repository to matching paths for all three audit scopes, independently of the `steer` or `log` run mode. Paths are normalized relative to the session's `cwd`.

Schema upgrades are transactional and stamped in `PRAGMA user_version` (`database.ts` `SCHEMA_UPGRADES`). Opening a database that still has the pre-schema `transcript_*` or `policy_replay_*` tables moves it to `<file>.pre-schema-<timestamp>` (with its WAL/shm sidecars), prints one warning, and starts a fresh database.

Upgrading from releases that kept two files is automatic. The first open of a missing `rerail.sqlite` beside a `rules.sqlite` (the former machine rule catalog) starts it as a transactionally consistent copy of that file, so every rule, project, and repository assignment carries over; `rules.sqlite` is left in place and never read again. A checkout's former `<checkout>/.rerail/transcripts.sqlite` is left untouched: its sessions stay there and are not imported, and any rules in its pre-catalog `rules` table are imported once into the repository's project, as before.

`rerail reset` deletes the current repository's session history, across all of its worktrees — ended sessions and every session's events, work links, audits, audit requests, and repository audits — in one write transaction. Rules, other repositories' history, audit settings, and presentation preferences stay. The file is never unlinked, so a daemon or harness session holding it open is unaffected, and a still-running session keeps its row (its earlier events are gone) and carries on recording. `just rerail-reset` runs it in a checkout's development container.

`rerail seed` fills the database with throwaway mock rows — three file rules covering `forbid` with regex, ast, and llm clauses, two conversation rules (one disabled), and a session with events and two work links attributed to the current repository — so MCP and UI work can proceed before the rule generator exists. It is idempotent.

## Install

From a nera-landing checkout:

```sh
just rerail
```

Prerequisites:

- Membership in the `nera-software` Azure DevOps organization, granted through the `Nera Developers` Entra group. Ask Jeremy (jbarnes@nera.software) to add you.
- The Azure CLI (`az`).
- Node.js 24+.
- `just`, the repository's command runner ([casey/just](https://github.com/casey/just)). Install it with `brew install just` (macOS), `cargo install just`, or your distro's package, and run recipes from the repo root where the `Justfile` lives.

`@neralabs/rerail` is published to the private Azure Artifacts feed `nera-software`. `just rerail` walks you through `az login`, mints a personal read-only Azure DevOps token (Packaging: Read), stores it in `~/.npmrc`, and verifies it (if Azure DevOps will not issue the token, it tells you how to get into the org or create the token by hand; `NERA_NPM_TOKEN` supplies one without a terminal); then it installs Rerail under `~/.rerail`, registers it with every coding harness on the machine, and starts the daemon. `rerail update` shells out to `npm` and uses the same `~/.npmrc`. When the token expires, npm commands report that the registry credentials are missing or expired; re-run `just rerail` to refresh them. `docs/installation.md` describes what setup writes, the manual token fallback, `rerail install` for harness registration on its own, and the checkout workflow. `just rerail-smoke` proves the whole path against a throwaway local registry without publishing anything. `just nuke-rerail` removes Rerail from the machine into a dated backup, and `just rerail --restore` reinstalls and restores that backup's databases; see `docs/installation.md`. `rerail install --remove` takes Rerail out of every coding agent, stops the daemon, and moves `rerail.sqlite` into the same kind of backup so rules and history are kept.

The CLI binary and launcher are `rerail` (`~/.rerail/bin/rerail`); setup deletes an older install's generated `bin/rerail` launcher and there is no `rerail` alias. Rerail stays off in a repository until you run `/rerail-start` or `rerail start` there.

To run this checkout's own build instead of the published package:

```sh
just rerail-dev
just rerail-dev --install-bridge # explicitly register the portable host bridge once
just rerail-dev --restore        # restore a just nuke-rerail backup's Rerail database into the container
```

`just` is the repository's command runner ([casey/just](https://github.com/casey/just)); install it with `brew install just` (macOS), `cargo install just`, or your distro's package, and run recipes from the repo root where the `Justfile` lives.

It builds and runs this checkout's daemon in an isolated Linux container. Each enabled workspace persists one distinct reserved `NERA_RERAIL_HOST_PORT` and matching loopback `NERA_RERAIL_PUBLIC_URL`; Compose maps that host port to private container port 4737, and the daemon, page, MCP, and API all use that one public workspace origin. Worker ports remain hidden, so concurrent workspaces do not contend for a second public origin. Docker with Compose v2, Git, Bash, Python 3, Node.js 24+, and `just` are required; Windows development uses WSL2. The Rust kernel build remains in the container, and the `nera-data-layer` submodule is not needed. Normal startup never rewrites global harness settings. The explicit `--install-bridge` step registers immutable JavaScript-only adapters that route each session to its enabled worktree; restart harness sessions after installing it. Installed or otherwise non-worktree Rerail continues to default to `http://127.0.0.1:4737`.

Rerail is off in every repository until it is started there. OMP, Pi, Claude Code, and Cursor use the same spellings: `/rerail-start` starts Rerail in the current checkout and the worktrees that follow it (from the main checkout, every linked worktree without its own setting) and `/rerail-stop` stops them, immediately detaching the active runtime; in Codex use `rerail start` / `rerail stop`. OMP and Pi register these commands natively. Claude Code and Cursor receive named command files during `rerail install`; Claude Code also receives `/rerail-ui` and `/rerail-session`, and Cursor `/rerail-ui`, prompt commands that open the Rerail page as a tab in the Orca or Cursor window before falling back to the system browser. `/rerail-settings` edits saved defaults through native selectors; `/rerail-scan` prompts for one-shot options. Maintenance commands are `/rerail-status`, `/rerail-check`, `/rerail-update`, `/rerail-rollback`, and `/rerail-reset` (with confirmation). Restart the harness process after installing or rebuilding to refresh command registration. Claude Code also retains the general `/rerail` launcher guidance at `~/.claude/commands/rerail.md`, written by `rerail install` and removed by `rerail install --remove`. Codex cannot expose an exact third-party slash-command name; run `rerail start` or `rerail stop` in its terminal. See [Scan and settings controls](#scan-and-settings-controls) for targeting and host differences.

Supported native targets:

- macOS arm64 and x64.
- Linux glibc arm64 and x64.
- Windows arm64 and x64 (MSVC).

Host entrypoints:

- OMP: `dist/default-extension.js`, declared in the package's `omp` manifest.
- Pi: `dist/pi-extension.js`, declared in the package's `pi` manifest and loadable with `pi -e`.
- Persistent event stream: `dist/event-stream.js` or `rerail events`.
- Claude Code and Codex: `createClaudeEventHandler(...)` and `createCodexEventHandler(...)` for harness-owned hook registration.
- Cursor: `.cursor-plugin/plugin.json`, backed by `dist/cursor-hook.js`.

Lifecycle ingestion is not an MCP tool surface. OMP and Pi own a session-scoped `RerailRuntime` inside their extension host. Claude Code, Codex, and Cursor command hooks send normalized events to the daemon's resident kernel over an owner-only Unix socket or a local Windows named pipe, so ending a hook process does not reset policy state. Harness-owned persistent integrations can also use the newline-delimited `RERAIL-EVENTS/1` stream. These paths share the same event runtime, cosine registry, and executable-policy bridge.

## Activate

Bind a machine to a signed customer profile:

```sh
rerail --profile acme activate \
  --code <one-use-code> \
  --activation-url https://activation.neralabs.com \
  --trust-root ./nera-activation-roots.json

rerail --profile acme profile status
```

For an air-gapped machine:

```sh
rerail --profile acme activation request \
  --deployment acme-airgap \
  --output activation-request.json

rerail --profile acme activate \
  --file acme-profile.jws \
  --trust-root ./nera-activation-roots.json
```

The trust-root file must arrive through an authenticated channel independent of the customer profile. Device private keys and returned credentials are stored in macOS Keychain, Linux Secret Service, or Windows Credential Manager. The signed profile selects the customer control plane and constrains automatic updates to its authorized npm dist-tag.

## Update

Updates are explicit. Nothing in Rerail checks a registry or installs a version on its own; `rerail events`, the hooks, and the daemon only ever run the active version.

```sh
rerail update status      # active and previous versions, channel, last check
rerail update check       # resolve the channel without installing
rerail update             # install and activate a newer version
rerail update rollback    # swap back to the previous version
```

`rerail update` first exports the rules of the current project (and of the project the daemon serves) to `<cwd>/.rerail/rules.json`, then resolves the configured npm dist-tag, installs the newer version below `~/.rerail/versions`, validates both the native runtime and event stream, and atomically activates it for newly started harness sessions. The prior version remains available for rollback. From a nera-landing checkout, `just rerail` re-runs setup against the registry; `just rerail-dev` instead rebuilds only that worktree's container.

## Release

The package the updater installs is built by `.github/workflows/rerail-release.yml` at the Landing repository root (GitHub reads workflows only from there, so nothing under `rerail/` can trigger a build). It compiles the native kernel for macOS, Linux glibc, and Windows MSVC on arm64 and x64, smoke-tests native loading and device storage on each host, rejects a Windows kernel that imports the Visual C++ runtime (the kernels link it statically, so a clean Windows install needs no redistributable), builds the runtime and bundled UI, refuses to pack unless all six `native/*.node` files are present, installs the tarball into a scratch prefix, and checks `rerail --help`, the bundled UI, and `nativeHealth()`. A Windows x64 job then installs that tarball through the real `just rerail` installer (`scripts/windows-install-smoke.ps1`: local Verdaccio, sandbox profile, captured output) and exercises the managed runtime; nothing is published unless it passes.

- `workflow_dispatch` builds and uploads the tarball as the `rerail-package` run artifact without publishing. Install it with `npm install <tarball>` to try a build.
- A `rerail-v<version>` tag (matching `package.json`) publishes `@neralabs/rerail` to the private Azure Artifacts feed (the registry of record, named by the `RERAIL_AZURE_NPM_REGISTRY` repository variable and authenticated with the `AZURE_DEVOPS_NPM_TOKEN` secret), under the dist-tag the tag names: `rerail-v0.2.0` → `latest`, `rerail-v0.2.0-beta.1` → `beta`, `rerail-v0.2.0-next.1` → `next`. The `rerail-` prefix keeps Rerail tags apart from Landing's own.
- The same tarball is then mirrored to npmjs with provenance when the `NPM_TOKEN` repository secret exists; without it that step is skipped with a notice, and earlier versions can be back-filled later from the `rerail-package` artifacts.

To cut a release, bump `rerail/package.json` only (`npm version <version> --no-git-tag-version` also updates `package-lock.json`). The Claude, Codex, and Cursor plugin manifests carry no version in the checkout: `npm pack` stamps `package.json`'s version into them (`prepack`) and restores the source form afterwards (`postpack`). If a pack fails partway, `node scripts/stamp-plugin-versions.mjs restore` puts them back.

## Run

One `rerail` install carries the rules kernel, the transcript database, and a browser view of it. `rerail daemon start` runs them as one background daemon:

```sh
rerail daemon start            # background daemon on http://127.0.0.1:4737/, opens nothing
rerail daemon start --open     # same, then opens the browser view
rerail daemon start --host 0.0.0.0 --allow-remote --public-host rerail.example   # bind off loopback (see below)
rerail start                   # start Rerail in this checkout and the worktrees that follow it; the first time also sets the repository up
rerail stop                    # stop Rerail in this checkout and the worktrees that follow it; leave the shared daemon running
rerail state                   # prints started, stopped, or never for this checkout
rerail status                  # whether Rerail is on here, then the daemon; exit 3 when the daemon is not running
rerail ui [--print]            # prints the rules page URL and opens it; --print only prints
rerail session [<host> <session-id>] [--open]   # this session's status line and page URL; --open opens it
rerail daemon stop             # graceful stop where supported; forced termination after 10 s
```

Before spawning, `rerail daemon start` probes the `better-sqlite3` binding under the Node that will run the daemon and rebuilds it in place when a Node upgrade has left it built for another ABI; a binding that still fails after the rebuild stops the launch with the reason, instead of the daemon crashing into `~/.rerail/daemon.log`. The same check runs when a harness session autostarts the daemon, and `rerail update` applies it to an installed version that fails validation before giving up on it.

The daemon owns the resident kernel execution service and exposes the Rerail database through HTTP, OpenAPI, and MCP. It writes rules and work links, while the shared runtime records harness events in the same database. It serves:

- `GET /healthz` — version, pid, database path and presence, native kernel status.
- `GET /openapi.json` — the OpenAPI 3.1 document for everything under `/api`; the web client is generated from it.
- `POST /mcp` — a stateless, JSON-response MCP Streamable HTTP endpoint exposing every `/api` operation as a tool of the same name. Non-POST requests return `405 Allow: POST`; there is no standalone SSE listener or transport session to terminate.
- `/api/*` — the operations below.
- Everything else — the bundled UI, with client-side routes falling back to `index.html`.

| Operation (HTTP and MCP tool) | Route                                                                 | Purpose                                                                                                                                                                                          |
| ----------------------------- | --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `rules_list`                  | `GET /api/rules?kind=&enabled=&applicability=&q=&source=&boundary=&mode=` | List or search: `kind` is `file` or `conversation`; `q` is a case-insensitive substring over id, reference/description, and action content or clause bodies; `mode` filters any rule; `source`, `boundary` filter conversation-rule samples. |
| `rules_activity`              | `GET /api/rules/activity?since=`                                      | What each rule caught since a time: distinct violations, sessions, last sighting, `audits` that evaluated it, and steering deliveries it drove.                                                |
| `rules_get`                   | `GET /api/rules/{id}`                                                 | One rule.                                                                                                                                                                                        |
| `rules_upsert`                | `PUT /api/rules/{id}`                                                 | Create or replace; body `{ kind, rule, enabled?, applicability?, revision? }`, validated against the kind's schema.                                                                              |
| `rules_set_enabled`           | `PATCH /api/rules`                                                    | `{ enabled, ids }` or `{ enabled, filter }` (same filter fields as `rules_list`) — "disable everything about X" is one call; returns rows changed.                                               |
| `rules_remove`                | `DELETE /api/rules/{id}`                                              | Delete one rule.                                                                                                                                                                                 |
| `rule_packs_list`             | `GET /api/rule-packs`                                                 | Every rule pack with its rules (kind, title, group, mode, on or off, and examples where the pack has them). Read-only. Listed rules carry `packId`. |
| `rule_packs_recommend`        | `GET /api/rule-packs/recommendations`                                 | The packs that fit this repository, each with its rule count and the reason it matched. Local; changes nothing.                                                                                 |
| `sessions_list`               | `GET /api/sessions`                                                   | Every recorded session with its event count.                                                                                                                                                     |
| `sessions_get`                | `GET /api/sessions/{sessionId}`                                       | One session summary.                                                                                                                                                                             |
| `session_events`              | `GET /api/sessions/{sessionId}/events?after=&limit=`                  | Ordered transcript events, at most 500 per page; `nextAfter` continues the read.                                                                                                                 |
| `work_links_list`             | `GET /api/sessions/{sessionId}/work-links`                            | External references linked to a session.                                                                                                                                                         |
| `work_links_add`              | `PUT /api/sessions/{sessionId}/work-links`                            | `{ kind, reference, url? }`; re-linking updates the url.                                                                                                                                         |
| `audit_settings_get`          | `GET /api/sessions/{sessionId}/audit-settings`                        | Ongoing audit defaults (`scope`, run `mode`) and the first `repositoryAudit` for the session's canonical workspace, with its `repositoryAuditRecord`.                                          |
| `audit_settings_update`       | `PATCH /api/sessions/{sessionId}/audit-settings`                      | Update the ongoing defaults; `mode` is `steer` or `log`.                                                                                                                                         |
| `audit_requests_create`       | `POST /api/sessions/{sessionId}/audit-requests`                       | Queue an explicit audit for one session.                                                                                                                                                         |
| `audit_requests_list`         | `GET /api/sessions/{sessionId}/audit-requests`                        | `{ auditRequests }`: the session's explicit audit requests and the `auditIds` they produced.                                                                                                     |
| `audits_list`                 | `GET /api/audits?auditId=&sessionId=&status=&scope=&mode=`            | `{ audits, total }`: durable audit records with their `violations` and failures; `mode` filters the run mode (`steer` or `log`).                                                                 |
| `violations_list`             | `GET /api/violations?since=&ruleId=&limit=&offset=`                   | Individual violations newest first, each with its `ruleId`, `clauseId`, session, and `auditId`.                                                                                                  |
| `repository_audits_list`      | `GET /api/repository-audits`                                          | One full repository audit per historically audited repository: its `repositoryAudit` and latest `audit` summary with `violationCount`.                                                          |
| `repository_audit_entries_list` | `GET /api/repository-audits/{auditId}/entries?kind=&mode=&code=&path=&q=` | Paginated violations and failures of one repository audit; `kind` is `violation` or `failure`, `mode` is the producing rule's mode.                                                    |

Every operation is defined once in `src/api/operations.ts` (id, route, zod schemas for params/query/body/output, handler). `src/api/http.ts` mounts them with Hono and emits the OpenAPI document; `src/api/mcp.ts` registers the same definitions as MCP tools, so a new operation appears in all three places with no further wiring. Validation failures return `400 { error: "invalid-request", issues }`; handler errors carry a status and a code (`rule-not-found`, `rule-id-mismatch`, `rule-conflict`, `repository-audit-not-found`). `rerail install` (and `rerail setup`) registers the endpoint with OMP by writing a `rerail` entry into `~/.omp/agent/mcp.json` (`{ "type": "http", "url": "http://127.0.0.1:4737/mcp" }`) next to the extension symlink; `--remove` takes only that entry out again. OMP lists it under `/mcp`; run `/mcp reload` if the daemon came up after the session did. A daemon started on another `--port` needs that entry edited to match. Other harnesses point at the same URL by hand.

The daemon binds `127.0.0.1` by default (`--port` and `--host` override). `--host` with anything but `127.0.0.1`, `localhost`, or `::1` is refused unless `--allow-remote` is also passed: off loopback, every host on the network could read and write the daemon. The container entrypoint is the one place that passes it, binding `0.0.0.0` inside the container's own network namespace behind Compose's `127.0.0.1` port mapping.

Being on loopback is not by itself the write authorization. A page on another site can resolve its own name to `127.0.0.1` (DNS rebinding) and reach the socket, so every request is first checked against the `Host` header the browser sent: it must name `127.0.0.1`, `localhost`, or `[::1]` on any port (a container port mapping or reverse proxy presents a port the daemon never bound), the literal address the daemon was bound to with `--host` (an IP literal cannot arrive by rebinding, and the daemon's own state URL dials it), or a hostname given with `--public-host <name,...>`. Anything else — including the private `/kernel/*` and `/native/*` bridges, `/mcp`, `/healthz`, and the page — is answered `421 { "error": "host-not-allowed" }` before any route runs. On top of that, an `/api` write carrying an `Origin` is accepted only when that origin is one of the same hostnames on the port the request was addressed to (`403 api-origin-forbidden` otherwise); Rerail's own processes send no `Origin`. The page is served with `X-Content-Type-Options: nosniff`, and its HTML with `Content-Security-Policy: frame-ancestors 'none'` and `X-Frame-Options: DENY`, so no other site can frame it.

The daemon's pid, port, and database path live in `~/.rerail/daemon.json` (or `$RERAIL_HOME/daemon.json`) and its output in `daemon.log` beside it. `rerail daemon start` reuses a daemon that is already running and reports where it is; a state file left by a crash or reboot is detected and cleared. `stop`, `status`, and `ui` always run from the launcher you invoked rather than re-executing the managed version, so they keep working while an update is in flight.

`daemon.json` also records `kernelSocketPath` (a Unix socket or Windows named pipe). Hooks reuse this service by default; `RERAIL_KERNEL_SOCKET` selects an explicitly managed endpoint. `RERAIL_AUTOSTART=0` prevents automatic daemon creation and requires an existing daemon or explicit endpoint for command hooks. A daemon started before resident IPC support must be stopped with `rerail daemon stop` and restarted before new hooks can use it; the stop command accepts the older state format.

Native tool-call IDs correlate edits with their results, including concurrent calls to the same tool. Events are ordered in a session-specific lane: waiting on one agent's open, audit, or delivery does not hold up unrelated sessions, while events within each session remain serialized. The resident kernel keys these lanes by runtime configuration and session ID, so separate databases or workspaces do not share a lane even when session IDs coincide. Pending calls and their captured preimages survive daemon restarts. Turn checkpoints discover filesystem changes made through shells or tools without explicit edit paths. Discovery retains source fingerprints, not a repository-sized copy of source text or a baseline of violations.

OMP uses its awaited `tool_call` hook to capture pre-edit source before the tool runs and `tool_result` to submit post-edit source for background evaluation. The asynchronous `tool_execution_start` and `tool_execution_end` notifications are not execution barriers and are not used for edit correlation.

An audit has three scopes and two independent run modes:

| Scope           | Reportable code                                                                                                            |
| --------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `repository`    | All eligible repository code, including existing violations; used for diagnostics and the initial journey.                 |
| `touched-files` | Entire touched files, including pre-existing violations within those files.                                                |
| `ai-output`     | Added or changed post-image lines, determined from captured pre-edit source. Untouched regions do not contribute violations. |

The `steer` run mode records results and sends violations of `steer` and `block` rules to the agent; violations of rules in `log` mode are only recorded. The `log` run mode records the same results without conversation output. There are no acknowledged violations, violation baselines, or already-seen suppression. Changing relevant source can produce the same violation again.

`RerailRuntime` accepts ongoing `policy: { scope, mode }` defaults; `NativeExecutablePolicyEnforcer` accepts the same settings directly. The first time Rerail sees a canonical workspace in local storage, it starts one independent full-repository audit in `log` mode. Later sessions in that workspace start without repeating that automatic audit. User-requested scans are also fixed to `log` mode and choose between the full eligible repository or the tracked, staged, and untracked files in the current Git diff.

AI-output scope uses source diffs, not differences between old and new violation IDs. A newly created file is entirely generated output. If pre-edit source was not captured, including an existing file changed through an untracked shell write, AI-output evaluation records a scope failure rather than silently widening to the entire file. Parsing and cross-file query context may be broader than reporting scope. Source, parse, and match caches remain incremental; repository scans stream bounded source batches through a separate native workspace.

Results carry source revisions. Empty checkpoints do not invalidate buffered violations; changes to relevant source do. OMP and Pi support background delivery, while polling hook hosts receive buffered violations at their next event. Delivery is not an audit: there is no acceptance callback, acknowledgement persistence, retry chain, or exactly-once guarantee.

An audit returns violations and concrete execution failures, not an `unknown` verdict. Unreadable sources, unavailable generated ranges, missing capabilities, exhausted budgets, and evaluator errors are stored in `audits`, never rendered as conversation observations. `audit_requests_create` submits an audit request for a session and `GET /api/audits` exposes paginated audit records.

OMP and Pi expose native argument-free commands with descriptions in slash completion. Claude Code and Cursor install the same `/rerail-start` and `/rerail-stop` names as user-level prompt commands; Claude Code also gets `/rerail-ui` and `/rerail-session`, Cursor `/rerail-ui`. Those prompt commands open the page as a tab in the Orca or Cursor window before falling back to the system browser:

| Command           | Interaction                                                                                                                        |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `/rerail-settings` | View saved ongoing defaults; select scope or mode to change it. Each selection saves immediately.                                  |
| `/rerail-scan`     | Choose `repository` or `git-diff`; the scan always runs log-only.                                                                  |
| `/rerail-start`   | Start Rerail in this checkout and attach it to the current session; the first time in a repository also sets it up.                |
| `/rerail-stop`    | Stop recording, evaluation, steering, and daemon autostart in this checkout. The shared daemon stays available to others.          |
| `/rerail-ui`       | Open the Rerail rules page in the browser (the same link the status line's rule count carries).                                     |
| `/rerail-session`  | Open this session's Rerail page in the browser (the same link `/rerail` prints).                                                     |
| `/rerail-status`   | Show whether Rerail is stopped here, otherwise daemon and installed version status.                                               |
| `/rerail-check`    | Check for an update without installing it.                                                                                         |
| `/rerail-update`   | Install a Rerail update.                                                                                                            |
| `/rerail-rollback` | Restore the previous installed version.                                                                                            |
| `/rerail-reset`    | Confirm before deleting this repository's Rerail session history, across all its worktrees. Rules are kept.                         |

Escape cancels a pending choice without saving it or submitting a scan; settings
already saved remain saved. Explicit scans cannot reconstruct AI-generated edit
ranges, so AI-output is offered only as an automatic-scan default. Git-diff
scans include tracked, staged, and untracked changed files while respecting Git
ignore rules. `/rerail-scan` waits up to ten seconds, then reports the latest
request status without changing saved defaults. Both commands bind to the
session and checkout where the dialog opened.

Rerail is off in every repository until it is started there. Set-up and rules
belong to the repository; on and off belong to each checkout. A linked worktree
follows the nearest checkout of its repository that contains it and was turned
on or off, at any depth, and otherwise the main checkout, until it is started
or stopped itself. Starting or stopping a checkout also reaches every worktree
that follows it: the worktrees inside it without their own setting, and, from
the main checkout, every linked worktree without its own setting, wherever it
is on disk. Worktrees with their own setting, and checkouts that contain this
one, keep their state. The state is stored in `~/.rerail/rerail.sqlite`.
`/rerail-start` (or `rerail start`) in a repository that has never been set up
also does the first-time set-up and seeds the default rules. Stopping takes
effect immediately in OMP and Pi and before
the next one-shot Claude Code, Codex, or Cursor hook runs. It persists across
coding agent and daemon restarts, does not delete history or rules, and does
not stop the machine-wide daemon. In a stopped checkout no events are
recorded, no file or conversation rules run, no guidance is delivered, and hooks do
not autostart or contact the daemon. The footer in Claude Code and OMP reads
"not on here" while a daemon is running to say so. `/rerail-status` reports
whether Rerail is on in this checkout before the shared-daemon state.

Upgrading keeps everyone where they were: a repository with recorded history
counts as started, and a checkout that carried the old `.rerail/disabled` marker
counts as stopped (in a linked worktree, that worktree only). The marker is not
read afterwards.

The exact `/rerail-start` and `/rerail-stop` names are available in OMP, Pi,
Claude Code, and Cursor. Codex currently namespaces custom prompts rather than
allowing an exact third-party slash-command name, so its equivalent is the
terminal CLI spelling `rerail start` / `rerail stop`.

The terminal CLI retains `rerail scan` and `rerail settings` with flags. Outside
the harness, select a session explicitly; Rerail never guesses the most recent:

```sh
rerail settings --session <session-id>
rerail settings --session <session-id> --scope touched-files --mode log
rerail scan --session <session-id> --scope repository --wait
rerail scan --session <session-id> --scope git-diff --wait --json
```

Without `--wait`, the command prints the recorded request immediately.
`--wait` polls for up to ten seconds and prints its latest state; `--json`
prints the structured record. Exit code `0` means submission/completion
succeeded, `1` means a request or execution failed or was interrupted, and `2`
means the wait expired while the scan remained queued/running. Findings alone
do not turn successful policy execution into a runtime failure. Use the
reported request/run IDs in Diagnostics to inspect outcomes.

The daemon of the Rerail home serves every recorded session, whichever
repository ran it, so the commands work from any directory. The standalone UI gives hosts without native selector
components the same controls with one clear page per responsibility:
**Settings** for ongoing scope and enforcement mode, **Diagnostics** for
log-only scan submission and results, and **Sessions** for transcript events.
These pages use these OpenAPI operations:

| Operation                   | HTTP surface                                       |
| --------------------------- | -------------------------------------------------- |
| Effective settings          | `GET /api/sessions/{sessionId}/audit-settings`     |
| Save defaults               | `PATCH /api/sessions/{sessionId}/audit-settings`   |
| Request an audit            | `POST /api/sessions/{sessionId}/audit-requests`    |
| Inspect requests            | `GET /api/sessions/{sessionId}/audit-requests`     |
| Repository audit index      | `GET /api/repository-audits`                       |
| Filtered audit entries      | `GET /api/repository-audits/{auditId}/entries`     |

**Ownership and activation.** Defaults belong to the canonical workspace
checkout, not a conversation, a machine-wide preference, or the database's
other workspaces. Sessions in that checkout use the same persisted defaults.
Different worktrees remain isolated. Selecting a different transcript database
does not establish project membership or cross-repository rule sharing; those
are the separate NER-1454 rule-ownership contract. This surface does not invent
an alternate project/group identity or silently promote checkout settings.
Effective settings identify their workspace and storage location.

The ongoing defaults are automatic `touched-files` audits in `steer`
mode. Saved scope/mode changes affect subsequent automatic evaluations,
including running sessions; they do not rewrite a running evaluation or its
recorded results. Every repository in retained scan history receives one
automatic full-repository, `log`-only baseline. When Rerail becomes active in a
historical repository without that baseline, it creates the repository audit before
continuing with ordinary session evaluation. That automatic baseline is never
repeated. A later user-requested scan becomes the latest result shown on
Diagnostics without modifying saved defaults. History filters never change
evaluation settings.

**Scope and outcomes.** Explicit repository scans cover the entire eligible
workspace. Explicit Git-diff scans resolve tracked, staged, and untracked
changed files when the request is made; an empty diff is a failed request rather
than a silent full-repository scan. Automatic AI-output evaluation continues to
use captured preimages and changed ranges.
Full-repository work is guarded by a five-minute execution budget, a
100,000-file or 512 MiB aggregate source ceiling, 32-file/1 MiB native batches,
and a 20-failure reporting limit. Individual files may use the policy engine's
32 MB source allowance and 500 ms AST work window. A scan that reaches a
guardrail fails explicitly with one summary failure instead of continuing to
consume host resources or flooding Diagnostics.

The Diagnostics screen keeps one-shot requests and their results separate from
ongoing settings and transcripts. A completed scan with no violations is a
successful empty result, not a failure. Unreadable inputs and evaluator errors
are execution failures, not violations. Queued/running requests are not
completed results, and interrupted execution is not success. Audit requests
link to the audit IDs they produced; history retains the session, workspace,
source revision, and evaluated rule revisions. Historical records and work
cancelled before evaluation have no compiled-rule provenance; they are not
attributed to today's rules. A recorded catalog with no individual revisions
means it had no enabled file rules. Diagnostics indexes every repository in
retained session history and renders the latest repository audit's violations
and execution failures as separate accessible records. Entry type
(`kind=violation|failure`), rule mode (`mode=log|steer|block`, the mode of the
rule that produced a violation), exact code, path substring, and free-text
filters execute in the daemon with bounded pagination; the UI does not load or
filter a full result set in memory. The rule mode filter shares the words
`steer` and `log` with an audit's run mode, which `mode=steer|log` filters on
`/api/audits`.

**Host capabilities.** Only the runtime owning the selected session executes
its requests. OMP/Pi can deliver actionable results in the background. Claude
Code, Codex, and Cursor receive buffered guidance at their next supported hook
event; completing a scan is not proof that the host has consumed its guidance.
`log` records results without steering; `steer` records the same results and
requests supported guidance delivery for violations of `steer` and `block`
rules. Runtime failures remain diagnostics, not
violations or conversation guidance. A closed or unavailable session
cannot be replaced by another session to make a request succeed.

Cursor Read and Write hook phases can share a native tool-call ID; their normalized event identities remain distinct. When the CLI omits prompt and response hooks, Rerail incrementally reads the host-provided JSONL transcript with a durable byte cursor. Native prompt hooks remain authoritative when present. Transcript tool-use records are not converted into invented tool events.

A missing native addon does not stop the daemon: `status`, `/healthz`, and the UI header say the kernel is unavailable while transcripts stay readable.

## Operate

The `conversation_rules` tool in OMP and Pi supports:

- `add`
- `remove`
- `list`
- `test`

Rule mutations and delivered-match state are session-scoped and replayable: OMP and Pi append native session entries; event-loop adapters append to the JSONL session ledger.

The `/cosine-rules` command supports:

```text
/cosine-rules list
/cosine-rules model
/cosine-rules reload
/cosine-rules test <rule-id> <text>
```

Use `test` to tune references and thresholds against representative positive and negative examples before enabling delivery.

## Semantic workspace

The lower-level programmatic interface separates four operations:

- `mark` attaches application-defined metadata to stable content references.
- `select` resolves mark expressions plus source, owner, session, and time filters.
- `project` renders and embeds selected items, groups, windows, or a whole selection with explicit budgets and lineage.
- `relate` compares projections with all-pairs, per-vector top-k, or corresponding-group pairing and optional reductions.

These operations are building blocks for custom workflows, not a finished multiplayer coordination interface. In particular, marks are local application data unless the caller supplies shared storage and synchronization.

```ts
import {
  bundledModelPath,
  NativeSemanticEmbedder,
  OmpMessageContentSource,
  OmpWatermarkStore,
  SemanticWorkspace,
} from "@neralabs/rerail";

const entries = () => ctx.sessionManager.getBranch();
const semantics = new SemanticWorkspace({
  sources: [
    new OmpMessageContentSource({
      sessionId: ctx.sessionManager.getSessionId(),
      entries,
    }),
  ],
  watermarks: new OmpWatermarkStore({
    entries,
    appendEntry: (customType, data) => pi.appendEntry(customType, data),
  }),
  embedder: new NativeSemanticEmbedder(bundledModelPath()),
});

const recent = await semantics.select({
  from: { owners: ["user"] },
  order: "time-descending",
  limit: 1,
});

await semantics.mark({
  kind: "attach",
  targets: recent.items.map((item) => item.ref),
  watermarks: [
    {
      namespace: "handoff",
      key: "requirement",
      value: true,
      owner: "engineer",
      visibility: "private",
      createdAt: Date.now(),
    },
  ],
});
```

`ContentRef.span` addresses a UTF-16 text range within an item. Marks are exact-target: a mark on a whole item does not implicitly apply to its spans. Expired marks remain in the append-only ledger but are excluded from selections. `visibility` is selection metadata, not an access-control mechanism.

OMP and Pi expose the semantic operations as native tools:

- `rerail_mark`
- `rerail_select`
- `rerail_project`
- `rerail_relate`

Claude Code, Codex, and Cursor lifecycle adapters are not agent-visible tool surfaces. Harness registrations call the event-loop interface directly.

## Design principles

### Outside the generative model

Inspection must not depend on an agent deciding to invoke Rerail. Host adapters capture supported events automatically.

### Local by default

The model, vectors, and compiled references remain local. Runtime matching makes no network requests.

### Evidence before action

A match must retain its rule, source, boundary, score, threshold, turn, and model fingerprint. Downstream behavior should be auditable and replayable.

### Semantics for attention, determinism for enforcement

Cosine similarity can establish relevance. It cannot reliably prove contradiction, compliance, authorization, ownership, or correctness.

### Shared context without prompt saturation

Store team knowledge outside the prompt and deliver only the context relevant to the current event.

### Human authority

Potential overlap is a coordination signal. Humans and deterministic team policies decide whether to notify, acknowledge, redirect, or block work.

## Landing monorepo boundary

Rerail is tracked directly under `rerail/` but is not a Landing application workspace or deployment dependency. Landing source must not import Rerail, and Rerail's runtime (`src/`) must consume Landing behavior only through declared API contracts.

Declare each consumed Landing operation and its contract/provider paths in `landing-api-dependencies.json`. Those paths participate in Rerail CI change detection.

The browser view is Rerail's own: `rerail/ui` is plain HTML, CSS, and ES modules with no framework and no build step (see "UI" below). Only the generated colour tokens cross from Landing, so changes to Landing's theme files also run the Rerail CI job.

From the Landing repository root:

```sh
npm run build:rerail
npm run check:rerail
npm run test:rerail
```

## Develop

```sh
npm ci
npm run build
npm run check
npm test
npm run test:integration
npm run benchmark
npm run package:check
```

Host-side `npm run build` runs `build:native`, which needs `~/.cargo/bin` on `PATH`: `scripts/rerail-native.sh` installs rustup without touching the shell profile, so `export PATH="$HOME/.cargo/bin:$PATH"` after it runs. Stop a running development container with `just rerail-down` before host builds delete output mountpoints. `just rerail-dev` needs no host Rust installation; its compiler and dependencies stay in the container.

Everything run from a checkout (tests, benchmarks, `node dist/cli.js`, the `just rerail-dev` container) is a development build: its policy content store is `<checkout>/.rerail/content-v3.sqlite`, private to that checkout or worktree. Only the managed installation under `~/.rerail/versions/` opens the machine-wide `~/.cache/nera/rerail/content-v3.sqlite`, so a checkout on a newer store schema cannot break the installed Rerail. `RERAIL_CONTENT_STORE_PATH` overrides the location; the test setup gives every test file its own store that way. See [Installing Rerail](docs/installation.md#after-setup).

Rerail's own toolchain is Node only. OMP remains an external Bun-hosted runtime; its integration suites (`integration/omp-*.test.ts`, `integration/harness-auth.test.ts`) use Vitest to drive a real standalone OMP executable. Set `RERAIL_OMP_EXECUTABLE` to its absolute path, or provision the release pinned to the SDK dependency with `node scripts/install-omp-host.mjs` (checksum-verified; uses a private local cache and prints the executable path). CI uses the same host. `npm run test:integration:built` skips the build step when `dist/` is current.

The benchmark gates p95 latency at 10 ms for one embedding plus a scan across 100 resident rules. `scripts/prepare-model.py` reproduces the bundled INT8 artifact from the revision and checksums recorded in `model/manifest.json`. The repeatable executable-policy benchmark for the full nera-data-layer rule corpus is documented in [Rerailer policy latency benchmark](docs/rerailer-benchmark.md).

### Container development loop

From a Landing checkout, run the development lifecycle:

```sh
just rerail-dev
# Open the printed workspace URL at /#rules — the same URL that serves its bundled UI
```

`just rerail-dev` runs this checkout's daemon in its per-worktree container. The
container daemon runs with `RERAIL_DEV=1`, which makes it serve the committed page
straight from the bind-mounted `rerail/ui`: edit a file, reload the browser, no
rebuild and no restart. The launcher never registers harnesses, and the normal
installed daemon and production UI have no development execution endpoints or
launch action.

The daemon remains the workspace's public origin for the page, API, OpenAPI, and
MCP. Private container port 4737 and the worker ports are hidden implementation
details, not browser entry points. `just rerail-setup` reserves a distinct
`NERA_RERAIL_HOST_PORT` for each enabled workspace and persists its matching
`NERA_RERAIL_PUBLIC_URL`; Compose maps that host port to the daemon's private 4737.
All of that workspace's public Rerail traffic uses the one printed origin, while
other enabled workspaces may run concurrently on their own reserved origins.
Installed or otherwise non-worktree Rerail may default to
`http://127.0.0.1:4737`.

Each daemon remains the exclusive writer for its workspace database. A launcher
may stop a managed daemon only when persisted state and health identify the exact
same workspace and database; it refuses foreign or unverified owners and never
stops an unrelated workspace.

Ctrl-C leaves the container running; the page keeps being served by the daemon.
Rerun `just rerail-dev` to rebuild or recover, and `just rerail-down` to stop it.

### Live harness smoke (macOS)

Run the real-provider smoke as the final acceptance step, after the deterministic regression suite:

```sh
npm run test:e2e:harness -- --harness all
npm run test:e2e:harness -- --harness codex
```

The command builds the complete package first. Each harness gets a temporary workspace, private profile, resident daemon, and a probed `sandbox-exec` write boundary. The smoke uses actual installed CLIs and providers—not scripted models or hook substitutes. It requires a committed violating edit with a matching native tool result, an executable-policy diagnostic, restoration of the transaction helper, exactly one cosine delivery containing a private nonce, and an assistant reply quoting that nonce and diagnostic.

The CI Azure Codex and Cursor invocations set `RERAIL_SMOKE_ACCEPTANCE=transport`; Claude Code retains the default `full` acceptance. Transport mode remains a real CLI/provider session and still gates sandboxing, one lifecycle session, the completed violating tool result, in-budget evaluation, the single cosine delivery, and the executable-policy delivery. It deliberately excludes the model's subsequent file restoration and final prose because those are provider sampling behavior rather than the hook transport contract. Full mode retains every assertion above.

Prerequisites:

- macOS with `sandbox-exec`, and the selected harness CLIs installed. `RERAIL_CLAUDE_BIN` and `RERAIL_CURSOR_AGENT_BIN` can select non-PATH executables.
- OMP and Pi use an access-only snapshot of the logged-in OMP `openai-codex` account. Codex uses `AZURE_OPENAI_API_KEY` with `AZURE_OPENAI_BASE_URL` when present, or an access-only snapshot of its own login otherwise. OAuth tokens must remain valid for at least ten minutes; refresh credentials are not copied.
- Claude Code runs on Microsoft Foundry (`CLAUDE_CODE_USE_FOUNDRY=1`) and requires `ANTHROPIC_FOUNDRY_API_KEY`, the Azure AI Services key for the resource. `ANTHROPIC_FOUNDRY_BASE_URL` defaults to `https://nera-ai-dev.services.ai.azure.com/anthropic`, and `RERAIL_CLAUDE_MODEL` defaults to the `claude-sonnet-4-6` deployment. Cursor Agent requires `CURSOR_API_KEY`.

Each provider-backed session has a 180-second limit by default; `RERAIL_SMOKE_TIMEOUT_MS` can raise that hard limit for a slower provider. Output is bounded and redacted; disposable profiles and workspaces are removed after each run. The command emits one JSON evidence record per harness and exits nonzero for any missing prerequisite or failed assertion. Missing authentication is a failure, never a skipped pass. The existing user daemon and harness profiles are not replaced.

Claude's internal temporary directory is explicitly kept inside the sandbox with `CLAUDE_CODE_TMPDIR`; API retries are disabled so authentication failures surface promptly instead of consuming the smoke timeout.

### UI

The browser view lives in `rerail/ui` and has no toolchain: `index.html`, `app.css`, and native ES modules (`app.js`, `api.js`, `state.js`, `render/*.js`), served as bytes. No React, no Vite, no Tailwind, no TypeScript, not even as dev dependencies. Colours come from `tokens.generated.css`, which a generator derives from Landing's `web/src/index.css`; nothing else crosses the boundary.

The standalone `neralabs/rerail` mirror is the tracked `rerail/` tree at one revision and nothing else. From Landing, run `python3 scripts/export_rerail_repository.py /path/to/rerail-checkout --revision HEAD --preserve .github` (Python 3.9+). The mirror carries the generated artifacts (`src/terminal-brand.generated.ts`, `ui/tokens.generated.css`) as tracked files; the generators leave them untouched when no Landing checkout is present, and PR CI in Landing fails if they are stale.

In a standalone mirror checkout, `npm ci && npm run build` needs only Node 24+, npm, and Rust; no sibling Landing files or parent `node_modules` are needed. The sync workflow builds the projection before pushing. The mirror preserves its own `.github` automation; this source projection does not update its release workflow. Make UI changes in Landing, not in the mirror.

`npm run build:ui` copies `ui/` to `dist/ui`, which the daemon serves. A daemon started with `--dev`, or with `RERAIL_DEV=1` in its environment, serves `ui/` itself instead, so an edit needs only a browser reload; `rerail status` and `/healthz` report it as `dev` rather than `bundled`. Use `just rerail-dev` for that; never `node dist/…` directly.

## License

MIT. The bundled `minishlab/potion-base-8M` model is also MIT-licensed. The opt-in TSLint rule pack
(`policies/executable-policy-dsl/rules/opt-in/tslint/`) is derived from palantir/tslint under Apache-2.0. See
`THIRD_PARTY_NOTICES` for both.
