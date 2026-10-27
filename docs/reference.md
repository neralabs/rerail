# Reference

How Rerail works in detail: rules and their configuration, the database, installation, updates, the daemon and its API, audits, and the internal-testing surfaces. Start with the [README](../README.md) for an overview.

## How rules work

Conversation rules are in [internal testing](../README.md#internal-testing). A conversation rule is a standing semantic query over one event source and boundary. The reference describes the concept to detect; the action contains the context to deliver.

The Rerail database (`$RERAIL_HOME/rerail.sqlite`) is the source of truth for rule configuration. A repository owns its configuration: rule packs, By Rerail rules and rules created by you are attached to it per folder, and rule settings turn rules or packs on or off, set their mode, and choose whether linters' suppression comments (`eslint-disable`, `noqa`, ...) silence them, for the repository or one checkout; a setting can also give a By Rerail code rule its option values. Each file is judged by the checkout that owns it; conversation rules come from the folder the session started in. OMP and Pi load the conversation rules at session start and re-read the configuration before every ingested event, and every runtime compiles the code rules into the kernel and re-reads them before each audit, so a change made through the daemon, the dashboard or MCP applies on the next event or tool call; the Rust kernel only loads and evaluates what the database hands it. Rules come only from the repository's configuration: there are no rule files in the working tree. See [Rule scoping](rule-scoping.md).

The package ships no rules. **By Rerail** rules are the rules of the rule packs you install, each its own npm package (`@neralabs/rerail-pack-<id>`) installed into `$RERAIL_HOME/packs`. They are read from the installed packs and never copied into the database; they run only where a repository adds them or a pack that holds them. Rules people write are **Created by you**, stored once per machine in the library. Editing a By Rerail rule stores a copy in the library, added where the original was with the original switched off there. `rerail rules export` writes the library and every repository's configuration to `$RERAIL_HOME/rules.json` (or `--output <file>`), and `rerail rules import <file>` loads one back. `rerail update` first copies the whole database to `$RERAIL_HOME/rerail.sqlite.pre-update`; `rerail reset` never touches rules.

Rule packs (`packs/<id>/`, [pack format](pack-format.md)) hold rule documents in `rules/` and a `pack.json` that lists each with its document, a title and a group, and a match definition (file globs, or dependencies in `package.json`, `requirements*.txt`, `pyproject.toml` or `composer.json`) that says which repositories it fits; a pack holds no code. Set-up fetches every Rerail pack from npm, matches each against the repository locally, with no model, and installs only the packs, at the versions shown, that the person confirms. `rerail packs list`, `add`, `remove` and `recommend` manage them afterwards. A pack for a project's own conventions is made the same way, with a `package.json` named `@neralabs/rerail-pack-<id>`, and installed with `rerail packs add <folder>`. See [Rule packs](rule-packs.md).

A code rule is a `rerail.policy/v1` document: a path `scope` and a list of regex, AST, or relational clauses the native engine evaluates, each with the violation it reports. The [code-rule DSL reference](code-rule-dsl-reference.html) documents every construct, and [Tool-call rules](tool-call-rules.md) covers structural matching of tool calls. A conversation rule file looks like this:

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

`mode` is required on every rule, conversation and code alike:

- `log` records a match for review and never sends guidance.
- `steer` sends the rule's guidance to the agent.
- `block` is stored and reported as `block`; until block enforcement lands, it is delivered exactly like `steer`. It is in [internal testing](../README.md#internal-testing).

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

Rerail keeps one SQLite file per Rerail home: `$RERAIL_HOME/rerail.sqlite` (`~/.rerail/rerail.sqlite` when `RERAIL_HOME` is unset). It holds every repository's rule configuration, the rules created by you, and the execution history of every repository on the machine; every harness adapter, runtime, and the daemon open it. The schema is a set of [Drizzle](https://orm.drizzle.team) tables in `src/engine/storage/schema.ts`; `npm run generate:schema` renders their DDL into `src/engine/storage/schema-sql.generated.ts` (`RERAIL_SCHEMA_STATEMENTS`, committed, checked for drift by `test/engine/catalog/rule-store.test.ts`), which `openRerailDatabase` applies to a fresh file. Rerail's database code runs only in Node processes and uses `drizzle-orm/better-sqlite3`. The OMP extension remains inside OMP's external Bun-based host; it never opens SQLite and reaches the database only through the daemon's native-session IPC.

- Rule configuration — `repositories`, `checkouts`, `rule_attachments`, `rule_settings`, `library_rules`, and their bookkeeping tables; see [Rule scoping](rule-scoping.md).
- `sessions` — one row per harness session (`host`, `cwd`, `repository_id`, `started_at`, `ended_at`). `repository_id` is the key of the checkout's repository (see [Rule scoping](rule-scoping.md)), and it is what attributes history to a repository.
- `events` — the append-only transcript for a session (`kind`, `observed_at`, `payload_json`, and a `payload_v8` blob when the payload is not JSON-representable).
- `work_links` — loose associations from a session to external references (`kind` such as `git-branch`, `github-pr`, `linear-issue`; `reference`; optional `url`). No foreign key on purpose, so links can be recorded by any process before or after the session row exists.
- `audits` — every run of rules over a session, live or re-run, with independent scope (`repository`, `touched-files`, `ai-output`) and run mode (`steer`, `log`), selected paths, session, source revision, timestamps, violations, and concrete execution failures. Terminal records are immutable; closing or reopening a session interrupts its unfinished audits without discarding partial results.

The daemon serves every repository on the machine. Its listings — sessions, audits, violations, repository audits, and rule activity — take an optional `repository` (a key or alias) and cover the whole machine without it; a session addressed by its id is found whichever repository recorded it. Code-rule path filters (`scope.include`, `scope.exclude`, exclusions winning) narrow a repository to matching paths for all three audit scopes, independently of the `steer` or `log` run mode. Paths are normalized relative to the session's `cwd`.

`rerail reset` deletes the current repository's session history, across all of its worktrees — ended sessions and every session's events, work links, audits, audit requests, and repository audits — in one write transaction. Rules, other repositories' history, audit settings, and presentation preferences stay. The file is never unlinked, so a daemon or harness session holding it open is unaffected, and a still-running session keeps its row (its earlier events are gone) and carries on recording.

`rerail seed` fills the database with throwaway mock rows — three code rules covering `forbid` with regex, ast, and llm clauses and two conversation rules, stored in the library and added to the current repository (which it sets up, with one conversation rule off), and a session with events and two work links attributed to that repository — so MCP and UI work has data to show. It is idempotent.

## Install

This section is the install guide, for you or for the coding agent you ask to install Rerail.

Run one command in the repository you want Rerail in:

```sh
npx @neralabs/rerail setup
```

From a Rerail checkout, `just rerail` runs the same set-up. To start in another repository, answer "Scan a different repo…" or pass `--repo ~/code/your-repo`.

- **Node 24 or newer.** On an older Node, set-up stops before changing anything and says how to upgrade.
- **What it does.** It installs Rerail into your home folder (`~/.rerail`), with no admin rights, and turns nothing on. Then it asks before reading the repository you ran it in, shows the rule packs that look right for it with the reason for each, and starts Rerail in that repository only once you say yes: every checkout of it that you have not stopped on its own. It connects the coding agents it finds (Claude Code, Cursor, Codex, OMP and Pi), or only the ones you name, and ends on a short summary and a link to the dashboard. Open a new session when you are ready; Rerail starts there.
- **Your choices.** Before the scan: scan this repo, scan a different repo, or skip the scan and pick rule packs yourself (Rerail then reads no code, and you can pick none). After it: start Rerail with the packs shown, change the rule packs, or not now (nothing is installed or turned on). Run outside a repository, it asks which folder to start with, or you can skip and later type `/rerail-start` in a session in any repository.
- **In a terminal**, answer each question there: move with ↑/↓ and press Enter, or type its number.
- **From a coding agent**, ask it to install Rerail and give it this section. A coding agent runs `npx @neralabs/rerail setup --relay`: set-up then prints each question for the coding agent to ask you as written, with the command to run for each answer (`rerail setup --relay --answer … --repo … --agents …`), even where the coding agent runs commands in a terminal. Without `--relay`, set-up asks this way whenever it has no terminal to ask in. The coding agent waits for your reply and runs the command for the answer you gave.
- **If your coding agent can't install it, run the same line in a terminal.**

Prerequisites:

- Node.js 24 or newer.
- macOS (arm64, x64), Linux glibc (arm64, x64), or Windows MSVC (arm64, x64). The package carries prebuilt native kernels for those targets; nothing is compiled on your machine.
- Read access to `@neralabs/rerail` on [npmjs](https://www.npmjs.com/), the only registry Rerail is published to. Until launch the package is restricted: sign in with `npm login` as a member of the `neralabs` npm organization (or put a read token in `~/.npmrc`). After launch it installs anonymously.

`setup` installs the package under `~/.rerail/versions/<version>` and points `~/.rerail/current` and the launcher `~/.rerail/bin/rerail` (`rerail.cmd` on Windows) at it, and puts `~/.rerail/bin` on `PATH` (a marked block in your shell's startup file, or the user `Path` on Windows; `RERAIL_MODIFY_PATH=0` leaves `PATH` to you); `rerail update` adds the entry too, where an earlier version's set-up never did. `rerail setup [--answer <answer>] [--repo <dir>] [--agents omp,pi,claude-code,codex,cursor] [--port <n>] [--json] [--relay]` takes the answers above as arguments; `rerail install --agents <list> [--dry-run]` connects coding agents without setting anything up. `rerail update` shells out to `npm` and uses the same `~/.npmrc`. `rerail install --remove` takes Rerail out of every coding agent and off `PATH`, stops the daemon, and moves `rerail.sqlite` into a dated backup so rules and history are kept. [Installing Rerail](installation.md) describes every set-up step, what it writes, how to remove Rerail, and the managed-installation layout.

Rerail is off in every repository until it is started there. Every coding agent gets one command set: `/rerail` (what Rerail is doing here, and every command), `/rerail-manage` (open the dashboard), `/rerail-create` (create a rule in your own words), `/rerail-start`, `/rerail-stop`, `/rerail-update` (install the newest version), `/rerail-uninstall` (remove Rerail from this computer; rules and history are kept in a backup), `/rerail-help` (the same as `/rerail`: a list of commands), and `/rerail-show-me` (see Rerail catch and fix a broken rule in this repo, then clean up). `/rerail-start` starts Rerail in the current checkout and the worktrees that follow it (from the main checkout, every linked worktree without its own setting) and `/rerail-stop` stops them, immediately detaching the active runtime. OMP and Pi register these commands natively. Claude Code and Cursor receive user-level command files in `~/.claude/commands/` and `~/.cursor/commands/` during `rerail install` (and `rerail setup`), removed by `rerail install --remove`. Codex receives one skill per command under `~/.codex/skills/<name>/SKILL.md`, invoked as `$rerail`, `$rerail-manage`, `$rerail-start`, and so on. Registration also removes the command files and skills earlier versions wrote under names no longer used (`/rerail-session`, and `/rerail-ui` and `/rerail-rage-quit` before they became `/rerail-manage` and `/rerail-uninstall`), when they are Rerail's own. `rerail update` then registers the active version again with every coding agent already connected (`rerail install --refresh`), so new and renamed commands arrive and retired ones go; a coding agent never connected stays untouched. Command files and skills Rerail generated are replaced without a `.bak` copy; a file of the same name that Rerail did not write is backed up first. Restart the coding agent after installing or rebuilding to refresh command registration. Settings, audits, status, the installed version, update checks, rollback, and reset are CLI commands (`rerail settings`, `rerail scan`, `rerail status`, `rerail version`, `rerail update check`, `rerail update rollback`, `rerail reset`). See [Scan and settings controls](#scan-and-settings-controls) for targeting and coding-agent differences.

Supported native targets:

- macOS arm64 and x64.
- Linux glibc arm64 and x64.
- Windows arm64 and x64 (MSVC).

Host entrypoints:

- OMP: `dist/hosts/omp/default-extension.js`, declared in the package's `omp` manifest.
- Pi: `dist/hosts/pi/pi-extension.js`, declared in the package's `pi` manifest and loadable with `pi -e`.
- Persistent event stream: `dist/harness/event-stream.js` or `rerail events` ([internal testing](../README.md#internal-testing)).
- Claude Code and Codex: `createClaudeEventHandler(...)` and `createCodexEventHandler(...)` for harness-owned hook registration.
- Cursor: `.cursor-plugin/plugin.json`, backed by `dist/hosts/cursor/cursor-hook.js`.

Lifecycle ingestion is not an MCP tool surface. OMP and Pi own a session-scoped `RerailRuntime` inside their extension host. Claude Code, Codex, and Cursor command hooks send normalized events to the daemon's resident kernel over an owner-only Unix socket or a local Windows named pipe, so ending a hook process does not reset policy state. Harness-owned persistent integrations can also use the newline-delimited `RERAIL-EVENTS/1` stream. These paths share the same event runtime, cosine registry, and executable-policy bridge.

## Activate

Bind a machine to a signed customer profile. Activation profiles are in [internal testing](../README.md#internal-testing).

```sh
rerail --profile acme activate \
  --code <one-use-code> \
  --activation-url https://activation.example.com \
  --trust-root ./activation-roots.json

rerail --profile acme profile status
```

For an air-gapped machine:

```sh
rerail --profile acme activation request \
  --deployment acme-airgap \
  --output activation-request.json

rerail --profile acme activate \
  --file acme-profile.jws \
  --trust-root ./activation-roots.json
```

The trust-root file must arrive through an authenticated channel independent of the customer profile. Device private keys and returned credentials are stored in macOS Keychain, Linux Secret Service, or Windows Credential Manager. The signed profile selects the customer control plane and constrains automatic updates to its authorized npm dist-tag.

## Update

Updates are explicit. Nothing in Rerail checks a registry or installs a version on its own; `rerail events`, the hooks, and the daemon only ever run the active version.

```sh
rerail version            # the installed version
rerail update status      # active and previous versions, channel, last check
rerail update check       # resolve the channel without installing
rerail update             # install and activate a newer version
rerail update rollback    # swap back to the previous version
```

`rerail update` first copies the Rerail database to `$RERAIL_HOME/rerail.sqlite.pre-update`, then resolves the configured npm dist-tag, installs the newer version below `~/.rerail/versions`, validates both the native runtime and event stream, and atomically activates it for newly started harness sessions. The prior version remains available for rollback. Then, whether or not it found a newer version, the active version registers itself again with every coding agent already connected (`rerail install --refresh`, which runs on its own too): the commands, skills and hooks it ships arrive and the ones it retired go, and a coding agent never connected is left alone. A connected coding agent that cannot be registered again is named with the reason, the others are still refreshed, and the update exits 1 with the new version active; one whose configuration cannot be read to tell whether it is connected is named and left alone. An update from a version without this step runs that version's update, so run `rerail install --refresh` once after it.

## Run

One `rerail` install carries the rules kernel, the transcript database, and a browser view of it. `rerail daemon start` runs them as one background daemon:

```sh
rerail daemon start            # background daemon on http://127.0.0.1:4737/ (a free port when 4737 is held), opens nothing
rerail daemon start --open     # same, then opens the browser view
rerail daemon start --host 0.0.0.0 --allow-remote --public-host rerail.example   # bind off loopback (see below)
rerail start                   # start Rerail in this checkout and the checkouts inside it (--repository: every checkout); in a repository that is not set up, first runs set-up's questions there
rerail stop [<host> [<session-id>]]   # stop Rerail in this checkout and the checkouts inside it (--repository: every checkout); leave the shared daemon running. A coding agent's host sets the reply's command sigil, and its session (rerail stop codex: CODEX_THREAD_ID) makes the reply name the rules that steered it
rerail state                   # prints started, stopped, or not-set-up for this checkout
rerail status                  # whether Rerail is on here, then the daemon; exit 3 when the daemon is not running
rerail manage [--print]        # prints the dashboard's Overview URL and opens it; --print only prints
rerail session [<host> <session-id>] [--open]   # this session's status line and page URL; --open opens it
rerail daemon stop             # graceful stop where supported; forced termination after 10 s
```

Before spawning, `rerail daemon start` probes the `better-sqlite3` binding under the Node that will run the daemon and rebuilds it in place when a Node upgrade has left it built for another ABI; a binding that still fails after the rebuild stops the launch with the reason, instead of the daemon crashing into `~/.rerail/daemon.log`. The same check runs when a harness session autostarts the daemon, and `rerail update` applies it to an installed version that fails validation before giving up on it.

The daemon owns the resident kernel execution service and exposes the Rerail database through HTTP, OpenAPI, and MCP. It writes rules and work links, while the shared runtime records harness events in the same database. It serves:

- `GET /healthz` — version, pid, database path and presence, native kernel status, and when Rerail was installed on this computer (`installedAt`).
- `GET /openapi.json` — the OpenAPI 3.1 document for everything under `/api`.
- `POST /mcp` — a stateless, JSON-response MCP Streamable HTTP endpoint exposing every `/api` operation as a tool of the same name. Non-POST requests return `405 Allow: POST`; there is no standalone SSE listener or transport session to terminate.
- `/api/*` — the operations below.
- Everything else — the bundled UI, with client-side routes falling back to `index.html`.

| Operation (HTTP and MCP tool) | Route                                                                 | Purpose                                                                                                                                                                                          |
| ----------------------------- | --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `repositories_list`           | `GET /api/repositories`                                               | Every repository Rerail knows: key, name, aliases, set up, started, checkout count. |
| `repository_context_get`      | `GET /api/repository?repository=&checkout=`                           | One repository with its clones and checkouts (own and effective state), and the checkout and folder of the target or calling agent. |
| `repository_rules_get`        | `GET /api/repository/rules?repository=&checkout=`                     | The repository's rules, each at its attachment's folder, with on, mode, suppressions, config and what decided each, when it was added there (`addedAt`) and, for a rule created by you, last edited (`updatedAt`); unresolvable references are listed as unavailable. |
| `repository_settings_set`     | `PUT /api/repository/settings`                                        | Turn rules or whole packs on or off, change their mode, or honour or ignore linters' suppression comments for them (`suppressions`), for the repository or one checkout (`owner`), optionally in one folder; set a By Rerail code rule's option values (`config`, whole repository only); null clears a value. |
| `repository_follow_gitignore_set` | `PUT /api/repository/follow-gitignore`                          | Follow `.gitignore` (on by default) or not: when on, files Git ignores are not audited (shell writes, direct edits, repository audits); tracked files always are. Sessions started after the change follow it; the repository summary carries `followGitignore`. |
| `repository_attachments_set`  | `PUT /api/repository/attachments`                                     | Add or remove rule packs, By Rerail rules and library rules per folder, with optional repository settings. The first addition (or `configure: true`) sets the repository up. |
| `repository_start`            | `POST /api/repository/start`                                          | Start Rerail with `scope` `checkout` (that checkout and the checkouts inside it) or `repository` (every checkout). |
| `repository_stop`             | `POST /api/repository/stop`                                           | Stop Rerail, with the same scopes. Rules and history are kept. |
| `repository_link`             | `POST /api/repository/aliases`                                        | Make another remote address resolve to this repository. People only. |
| `repository_reset`            | `POST /api/repository/reset`                                          | Clear the repository's history across all its checkouts, as `rerail reset` does: its sessions with their events, work links, audits and violations. Ended sessions are removed; running ones keep recording. Rules and settings are unchanged. People only. |
| `rules_activity`              | `GET /api/rules/activity?since=&repository=`                          | What each rule caught since a time: distinct violations (and how many were logged only), sessions, last sighting, `audits` that evaluated it, and steering deliveries it drove; `sessions: { started, withViolations }` counts the sessions that started since then and those with a violation reported since then. |
| `rule_packs_list`             | `GET /api/rule-packs`                                                 | Every rule pack with its rules (kind, title, group, mode, on or off, and examples where the pack has them). Read-only. |
| `rule_packs_recommend`        | `GET /api/rule-packs/recommendations?repository=&checkout=`           | The packs that fit a checkout, each with its rule count, the reason it matched and the folder to add it in. Local; changes nothing. |
| `coding_agents_list`          | `GET /api/coding-agents`                                              | Each coding agent Rerail works with: whether it is on this computer (`found`), whether Rerail is connected to it (`connected`), and any `problem`. A dry run; changes nothing. Settings shows it. |
| `rerail_rules_list`           | `GET /api/rerail-rules?kind=`                                         | The By Rerail rules, each with the pack it belongs to. |
| `library_rules_list`          | `GET /api/library-rules?kind=`                                        | The rules created by you. |
| `library_rules_get`           | `GET /api/library-rules/{id}`                                         | One rule created by you. |
| `library_rules_upsert`        | `PUT /api/library-rules/{id}`                                         | Create or replace a rule created by you (`attach: true` also adds it to the target repository); a By Rerail rule's id stores an edited copy instead. |
| `library_rules_remove`        | `DELETE /api/library-rules/{id}`                                      | Delete a rule created by you, with its attachments and settings. |
| `sessions_list`               | `GET /api/sessions?since=&limit=&cursor=`                             | Recorded sessions newest first, a page at a time, each with its event and violation counts, how many of its violations were steered rather than logged (`steeredViolations`), and when it last recorded an event (`lastEventAt`); `total`, and `nextCursor` for the next page. |
| `sessions_get`                | `GET /api/sessions/{sessionId}`                                       | One session summary, with `gaps`: events its coding agent never delivered and boundaries answered before checks ran, over the whole transcript. |
| `session_events`              | `GET /api/sessions/{sessionId}/events?after=&before=&limit=`          | Ordered transcript events, at most 500 per page: forward from `after`, or the page before `before`; `nextAfter` continues the read.        |
| `work_links_list`             | `GET /api/sessions/{sessionId}/work-links`                            | External references linked to a session.                                                                                                                                                         |
| `work_links_add`              | `PUT /api/sessions/{sessionId}/work-links`                            | `{ kind, reference, url? }`; re-linking updates the url.                                                                                                                                         |
| `audit_settings_get`          | `GET /api/sessions/{sessionId}/audit-settings`                        | Ongoing audit defaults (`scope`, run `mode`, experimental `steerStrategy`) and the first `repositoryAudit` for the session's canonical workspace, with its `repositoryAuditRecord`.                         |
| `audit_settings_update`       | `PATCH /api/sessions/{sessionId}/audit-settings`                      | Update the ongoing defaults; `mode` is `steer` or `log`, `steerStrategy` (experimental) is `message`, `rewind` or `subagent`.                                                                                   |
| `audit_requests_create`       | `POST /api/sessions/{sessionId}/audit-requests`                       | Internal testing. Queue an explicit audit for one session.                                                                                                                                       |
| `audit_requests_list`         | `GET /api/sessions/{sessionId}/audit-requests`                        | Internal testing. `{ auditRequests }`: the session's explicit audit requests and the `auditIds` they produced.                                                                                   |
| `audits_list`                 | `GET /api/audits?auditId=&sessionId=&status=&scope=&mode=`            | `{ audits, total }`: durable audit records with their `violations` and failures; `mode` filters the run mode (`steer` or `log`).                                                                 |
| `violations_list`             | `GET /api/violations?since=&ruleId=&sessionId=&limit=&cursor=`        | Individual violations newest first, each with its `ruleId`, `clauseId`, session, `host`, and `auditId`; `total`, and `nextCursor` for the next page. |
| `repository_audits_list`      | `GET /api/repository-audits`                                          | Internal testing. One full repository audit per historically audited repository: its `repositoryAudit` and latest `audit` summary with `violationCount`.                                        |
| `repository_audit_entries_list` | `GET /api/repository-audits/{auditId}/entries?kind=&mode=&code=&path=&q=` | Internal testing. Paginated violations and failures of one repository audit; `kind` is `violation` or `failure`, `mode` is the producing rule's mode.                                  |

Every operation is defined once (`src/api/operations.ts`, with the repository and rule operations in `src/api/repository-operations.ts`: id, route, zod schemas for params/query/body/output, handler). `src/api/http.ts` mounts them with Hono and emits the OpenAPI document; `src/api/mcp.ts` registers the same definitions as MCP tools, so a new operation appears in all three places with no further wiring. Validation failures return `400 { error: "invalid-request", issues }`; handler errors carry a status and a code (`rule-not-found`, `rule-id-mismatch`, `rule-conflict`, `repository-audit-not-found`, `target-required`, `target-mismatch`, `agent-not-permitted`, `invalid-cursor`). `rerail install` (and `rerail setup`) registers the endpoint with OMP by writing a `rerail` entry into `~/.omp/agent/mcp.json` next to the extension symlink: a stdio server that runs the installed `rerail mcp-bridge` (`{ "command": "<node>", "args": ["<install>/dist/cli/cli.js", "mcp-bridge"] }`); `--remove` takes only that entry out again. The bridge relays each request to the daemon that `daemon.json` (below) records, so it follows a daemon that took a free port, was started with `--port`, or restarted, and the entry never needs editing. It checks each `daemon.json` once, sending nothing to its daemon until that daemon's `/healthz` answers with the recorded pid, so a bridge that starts after a crash or a reboot left `daemon.json` behind answers "Rerail daemon is not running" instead of reaching whatever now holds the port. After that check a request costs one file read, and the bridge checks again only when a request fails to connect: if a daemon it already checked is killed with SIGKILL, its pid is reused and another daemon takes its port before the next request, that request reaches the other daemon. OMP lists it under `/mcp`; run `/mcp reload` if the daemon came up after the session did. Other harnesses can run the same bridge command, or read the daemon's URL from `daemon.json`.

Repository and rule operations name their target in the query or body: `repository` (a key such as `remote:github.com/acme/app` or `clone:<id>`, or an alias), `checkout` (an absolute path in a checkout) and `folder`. A checkout wins over a repository, and an MCP call without either uses the calling agent's checkout: the MCP bridge sends `x-rerail-caller-cwd`, the URI-encoded real path of the folder its session started in, on every request. `/api` callers are people and `/mcp` callers are coding agents (advisory, not a security boundary). A coding agent may start, stop and change settings in its own checkout and the checkouts inside it; it may change a repository's attachments and settings only while the repository is not set up, or to add a rule it creates in the same call or its copy of a By Rerail rule. Anything else is `403 agent-not-permitted` with the dashboard URL, `<origin>/?repository=<key>&checkout=<path>#rules`, where a person can make the change. See [Rule scoping](rule-scoping.md).

The daemon binds `127.0.0.1` by default (`--port` and `--host` override). `--host` with anything but `127.0.0.1`, `localhost`, or `::1` is refused unless `--allow-remote` is also passed: off loopback, every host on the network could read and write the daemon.

Being on loopback is not by itself the write authorization. A page on another site can resolve its own name to `127.0.0.1` (DNS rebinding) and reach the socket, so every request is first checked against the `Host` header the browser sent: it must name `127.0.0.1`, `localhost`, or `[::1]` on any port (a container port mapping or reverse proxy presents a port the daemon never bound), the literal address the daemon was bound to with `--host` (an IP literal cannot arrive by rebinding, and the daemon's own state URL dials it), or a hostname given with `--public-host <name,...>`. Anything else — including the private `/native/*` bridge, `/mcp`, `/healthz`, and the page — is answered `421 { "error": "host-not-allowed" }` before any route runs. On top of that, an `/api` write carrying an `Origin` is accepted only when that origin is one of the same hostnames on the port the request was addressed to (`403 api-origin-forbidden` otherwise); Rerail's own processes send no `Origin`. The page is served with `X-Content-Type-Options: nosniff`, and its HTML with `Content-Security-Policy: frame-ancestors 'none'` and `X-Frame-Options: DENY`, so no other site can frame it.

The daemon's pid, port, and database path live in `~/.rerail/daemon.json` (or `$RERAIL_HOME/daemon.json`) and its output in `daemon.log` beside it. `rerail daemon start` reuses a daemon that is already running and reports where it is; a state file left by a crash or reboot is detected and cleared. `stop`, `status`, and `manage` always run from the launcher you invoked rather than re-executing the managed version, so they keep working while an update is in flight.

`daemon.json` also records `kernelSocketPath` (a Unix socket or Windows named pipe). Hooks reuse this service by default; `RERAIL_ENGINE_SOCKET` selects an explicitly managed endpoint. `RERAIL_AUTOSTART=0` prevents automatic daemon creation and requires an existing daemon or explicit endpoint for command hooks.

Native tool-call IDs correlate edits with their results, including concurrent calls to the same tool. Events are ordered in a session-specific lane: waiting on one agent's open, audit, or delivery does not hold up unrelated sessions, while events within each session remain serialized. The resident kernel keys these lanes by runtime configuration and session ID, so separate databases or workspaces do not share a lane even when session IDs coincide. Pending calls and their captured preimages survive daemon restarts. Turn checkpoints discover filesystem changes made through shells or tools without explicit edit paths. Discovery retains source fingerprints, not a repository-sized copy of source text or a baseline of violations.

OMP uses its awaited `tool_call` hook to capture pre-edit source before the tool runs and `tool_result` to submit post-edit source for background evaluation. The asynchronous `tool_execution_start` and `tool_execution_end` notifications are not execution barriers and are not used for edit correlation.

### Scan and settings controls

Full-repository scanning (`rerail scan`, explicit audit requests, the automatic first repository audit and the repository audit operations) is in [internal testing](../README.md#internal-testing).

An audit has three scopes and two independent run modes:

| Scope           | Reportable code                                                                                                            |
| --------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `repository`    | All eligible repository code, including existing violations; used for diagnostics and the initial journey.                 |
| `touched-files` | Entire touched files, including pre-existing violations within those files.                                                |
| `ai-output`     | Added or changed post-image lines, determined from captured pre-edit source. Untouched regions do not contribute violations. |

The `steer` run mode records results and sends violations of `steer` and `block` rules to the agent; violations of rules in `log` mode are only recorded. The `log` run mode records the same results without conversation output. There are no acknowledged violations, violation baselines, or already-seen suppression. Changing relevant source can produce the same violation again.

Experimental, in [internal testing](../README.md#internal-testing): the workspace's `steerStrategy` decides how a code-rule steer reaches the agent, and takes effect only where `RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1` is set in the environment of the process that runs Rerail's runtime (the daemon for OMP, Claude Code, Codex and Cursor; the coding agent itself for a Pi session running Rerail in process). Without it every steer goes out as `message`, whatever is saved. `message` (the default, and what settings saved before it existed read as) sends it as its own message. `subagent` sends the same message ending with an instruction to hand the fix to a subagent with the host's subagent tool (`rerail_fix` in Pi, which Rerail registers; `task` in OMP; `Task` in Claude Code and Cursor; `spawn_agent` in Codex) and keep to the agent's own task. `rewind` rolls back an edit whose own check steers while its tool result is still being answered: every file the call captured goes back to its preimage (a file it created is deleted), all or none, and the steer, opening with what was put back and that the edit is to be redone following the rule, goes into that tool call's result. Rerail's rollback is audited in `log` mode, so the restored file steers nothing on its own. An edit whose preimage could not be read, a steer for a file the call did not capture, and a steer that arrives after the result was answered (a one-shot hook waits a second) are sent as a `message` instead. The session ledger's `delivery` entries record each steer's `strategy`, and a rolled-back edit's `toolCallId` and `rewound` paths.

`RerailRuntime` accepts ongoing `auditSettings: { scope, mode, steerStrategy }` defaults; `NativeCodeRuleEnforcer` accepts `scope` and `mode` directly. The first time Rerail sees a canonical workspace in local storage, it starts one independent full-repository audit in `log` mode. Later sessions in that workspace start without repeating that automatic audit. User-requested scans are also fixed to `log` mode and choose between the full eligible repository or the tracked, staged, and untracked files in the current Git diff.

AI-output scope uses source diffs, not differences between old and new violation IDs. A newly created file is entirely generated output. Pre-edit source is captured at the host's tool-call boundary; an edit made without one, such as a shell write or a Codex `apply_patch` that its hooks do not show, takes as its pre-edit source the text Rerail last saw the file with in this session (the most recently changed 64 MB of them), else its Git index or `HEAD` blob, either only when its SHA-256 equals the file's digest before the change; reading the blobs never fetches a partial clone's missing objects. Where none matches, the edit's generated lines are unknown, so that edit reports its whole file, as `touched-files` does; once Rerail has seen the file change in the session, its later edits report only their own lines again. Parsing and cross-file query context may be broader than reporting scope. Source, parse, and match caches remain incremental; repository scans stream bounded source batches through a separate native workspace.

Results carry source revisions. Empty checkpoints do not invalidate buffered violations; changes to relevant source do. OMP and Pi support background delivery, while polling hook hosts receive buffered violations at their next event. A newer edit of a file drops a steer still queued for it; in AI-output scope that edit then reports the lines generated since the dropped steer's edit along with its own, as it does for an earlier edit it superseded before evaluation, so a violation the agent was never shown reaches it once if the file still has it, and never if the newer edit removed it. A violation the agent was shown is not repeated by later edits elsewhere in the file. Delivery is not an audit: there is no acceptance callback, acknowledgement persistence, retry chain, or exactly-once guarantee.

An audit returns violations and concrete execution failures, not an `unknown` verdict. Unreadable sources, unavailable generated ranges, missing capabilities, exhausted budgets, and evaluator errors are stored in `audits`, never rendered as conversation observations. `audit_requests_create` submits an audit request for a session and `GET /api/audits` exposes paginated audit records.

OMP and Pi expose the commands natively, with descriptions in slash completion. Claude Code and Cursor install the same names as user-level prompt commands, and Codex as `$rerail-*` skills:

| Command            | Interaction                                                                                                                        |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| `/rerail`          | Show whether Rerail is on in this repository, its rule count, how often it steered this session, the session's dashboard link, and every command. Rendered by `rerail session`; Cursor gets the repository-level form. |
| `/rerail-manage`   | Open the dashboard's Overview for this repository and checkout (the same link the footer's rule count carries).                   |
| `/rerail-create`   | Create a rule in your own words, or with no words get suggestions from this repository's sessions (`rerail rules evidence`) and written standards: the coding agent follows the package's `skills/rerail-create` skill, drafts the rule, tests it with `rerail rules test <draft.json>`, reads it back, and adds it with `rerail rules add <draft.json>` only after you say yes: the rule goes into your library and is added to this repository (setting it up with that rule when it is not set up yet). It also tunes a rule that misfires. See [docs/writing-rules.md](writing-rules.md). |
| `/rerail-start`    | Start Rerail in this checkout and the worktrees inside it and attach it to the current session; in a repository that is not set up it first runs set-up's questions there (OMP and Pi ask through their own picker; Claude Code and Cursor relay the questions as text). |
| `/rerail-stop`     | Stop recording, evaluation, steering, and daemon autostart in this checkout and the worktrees inside it. The shared daemon stays available to others. |
| `/rerail-update`   | Install the newest Rerail version.                                                                                                 |
| `/rerail-uninstall` | Remove Rerail from this computer (`rerail install --remove`); rules and history are kept in a backup. Asks first: OMP and Pi use a native selector, and Claude Code does not pre-approve the removal. |
| `/rerail-help`     | The same as `/rerail`: whether Rerail is on here, then a list of commands.                                                         |
| `/rerail-show-me`  | A demo on this repository, under a minute: `rerail show-me` adds the demo rule `local:rerail-show-me`, scoped to one file, `rerail-show-me.js`; the coding agent writes that file breaking the rule, is steered, fixes it, and `rerail show-me --cleanup` deletes the file and takes the rule out of this repository, leaving it to a demo running in another until that one cleans up. Rerail must be on and the repository set up; the demo never starts or sets one up, and never touches a `rerail-show-me.js` or a `local:rerail-show-me` rule it did not write. The steer stays in the dashboard's History, as for any rule. |

A rule draft for `/rerail-create` is `{ kind, rule, cases: { caught: [...], passed: [...] } }`. `rerail rules test` validates it and runs its cases plus a pass over the repository without writing anything; a case whose file some clause could not check (it did not parse, say) is marked `not checked (<reason>)`, never passed, and the repository pass lists the files the rule could not check. `rerail rules add` stores it in your library and adds it, on, to this repository (the whole repository, or the draft's `folder`), setting a repository that is not set up up with just that rule; it refuses to replace a rule shipped by Rerail.

Rerail is off in every repository until it is started there. Rules belong to
the repository; on and off belong to each checkout, else the nearest checkout
of the same repository that contains it and has its own value, else the
repository. Starting or stopping a checkout also reaches the checkouts inside
it, which follow it again; `rerail start --repository` and `rerail stop
--repository` (or the dashboard) act on every checkout of the repository. The
state is stored in `~/.rerail/rerail.sqlite`. `/rerail-start` (or `rerail
start`) in a repository that is not set up first runs the set-up there: it
asks, recommends rule packs, and once you confirm adds them (a language pack
at the folder it matched, a repository pack for the whole repository) and
starts. A started repository that is not set up records
sessions but runs no rules. Stopping takes effect immediately in OMP and Pi
and before the next one-shot Claude Code, Codex, or Cursor hook runs. It
persists across coding agent and daemon restarts, does not delete history or
rules, and does not stop the machine-wide daemon. In a stopped checkout no
events are recorded, no code or conversation rules run, no guidance is
delivered, and hooks do not autostart or contact the daemon. The footer in
Claude Code, OMP, and Pi reads `R E R A I L ○ not on here` while a daemon is
running to say so, and `R E R A I L ○ not set up · rerail start` in a started
repository that is not set up. `/rerail` and `rerail status` report whether
Rerail is on in this checkout.

The terminal CLI retains `rerail scan` and `rerail settings` with flags. Outside
the coding agent, select a session explicitly; Rerail never guesses the most recent:

```sh
rerail settings --session <session-id>
rerail settings --session <session-id> --scope touched-files --mode log
RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1 rerail daemon start   # experimental: the daemon honours the strategy
rerail settings --session <session-id> --steer-strategy rewind
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
does not share rules across repositories; rule ownership is the repository's
(see [Rule scoping](rule-scoping.md)). This surface does not silently
promote checkout settings.
Effective settings identify their workspace and storage location.

The ongoing defaults are automatic `ai-output` audits in `steer` mode, steered
with the `message` strategy: an edit is told only about violations in the lines
it added or changed, not about what the rest of its file already broke.
`--scope touched-files` reports whole edited files instead. A workspace whose
settings were saved keeps the scope they name. Saved scope/mode/strategy changes
affect subsequent automatic evaluations,
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
100,000-file or 512 MiB aggregate source ceiling, and 32-file/1 MiB native
batches; a scan that reaches one of these fails explicitly with one summary
failure, always listed, instead of continuing to consume host resources.
Failures to read a file never stop a scan, and at most 20 of them (and of other
per-file failures) are listed; the audit record and the session summary keep
the total (`failureCount`, of which `unlistedFailureCount` are not listed), and
the session footer says `audit failed (N failures)`. Individual files may use
the policy engine's 32 MB source allowance.
No per-file limit is a fixed cut-off for a large file. Each file's shared work
(parsing, its tape and anchor index) and each clause's own work allowance grow
with the file's size, and a clause may visit `maxVisitedNodes` nodes or 16 per
node of the file (`maxVisitsPerNode`), whichever is more. Each clause runs
under its own allowance, so a costly rule fails alone and never takes the
other rules' findings on that file with it. In one evaluation of a file, its
clauses together may spend eight allowances; a clause that finds that ceiling
spent fails without running. A rule running out of its own allowance in an
evaluation of all of a file's rules depends on its content and the rules alone,
so the result is stored and a later scan reads it back instead of paying for it
again. A failure that depends on what else ran is not stored, and the next scan
evaluates the file or rule again: the file's shared work running out (how much
of it a scan does depends on what an earlier one stored), the ceiling, or
running out in an evaluation of some of its rules.

A rule that still runs out of work on a file is a **coverage gap**, not an
execution failure: its failure says what would let it finish (simplify the
rule, or exclude generated or vendored files from its scope), the failure list
keeps one failure of every such rule past its cap, and the session footer
counts the rules that ran out of work. A rule the file's work stopped (its
shared work or the ceiling; code `file-budget-exceeded`) is a gap too, advised
to exclude or split the file, and is not counted as a rule that ran out. A file
every rule failed on is a **blind file**: it is listed once, as one failure
marked `blind`, never cut from the list, and the audit record reports
`blindFiles` and `blindFileCount`. A file blind for its work (`budget`) is a
gap; one blind for its syntax (`syntax`) is a file read in part, as a rule kept
out by syntax errors is: a repository audit lists it and completes, a
touched-files scan fails on it. A scan whose only failures are gaps completes,
`rerail scan` exits `0`, and the session footer says how many files were not
checked, how many rules ran out of work and how many the file's work stopped
instead of "audit failed". A rule that finds more than `maxWitnesses` on a file
(`witness-limit`) did read it, and a file past a rule's input limits
(`input-limit`) was never read by it: neither is a gap, and both fail the audit.

The Diagnostics screen keeps one-shot requests and their results separate from
ongoing settings and transcripts. A completed scan with no violations is a
successful empty result, not a failure. Unreadable inputs and evaluator errors
are execution failures, not violations. Queued/running requests are not
completed results, and interrupted execution is not success. Audit requests
link to the audit IDs they produced; history retains the session, workspace,
source revision, and evaluated rule revisions. Historical records and work
cancelled before evaluation have no compiled-rule provenance; they are not
attributed to today's rules. A recorded catalog with no individual revisions
means it had no enabled code rules. Diagnostics indexes every repository in
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

The `conversation_rules` tool and `/cosine-rules` are in [internal testing](../README.md#internal-testing).

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

The semantic workspace and its OMP and Pi tools are in [internal testing](../README.md#internal-testing).

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
