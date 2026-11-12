# Reference

Rerail in detail: rules, the database, install, updates, the daemon and its API, audits, and internal-testing surfaces. Start with the [README](../README.md) for an overview.

## How rules work

Rules live in the Rerail database, attach to repositories per folder, and run as code rules or conversation rules.

- **Rerail database** (`$RERAIL_HOME/rerail.sqlite`): the source of truth for rule configuration. There are no rule files in the working tree.
- **Attachments** add rule packs, By Rerail rules and rules created by you to a repository, per folder.
- **Rule settings** turn rules or packs on or off and set their mode, for the repository or one checkout.
- **Code rule**: a path scope plus clauses the native engine evaluates against source.
- **Conversation rule** ([internal testing](../README.md#internal-testing)): a standing semantic query over one event source and boundary. Its reference describes what to detect; its action, the context to deliver.

Each file is judged by the checkout that owns it; conversation rules come from the folder the session started in. See [Rule scoping](rule-scoping.md).

<details>
<summary>More settings, and when changes apply</summary>

Rule settings also choose whether linters' suppression comments (`eslint-disable`, `noqa`, ...) silence a rule, and can set a By Rerail code rule's option values.

Changes made through the daemon, dashboard or MCP apply on the next event or tool call. OMP and Pi load conversation rules at session start and re-read configuration before every ingested event. Every runtime compiles code rules into the kernel and re-reads them once per event, before any of its audits, and again when a scan starts. The Rust kernel only loads and evaluates what the database hands it.

</details>

### Where rules come from

The package ships no rules. Each rule is one of:

- **By Rerail**: from a Rerail rule pack, an npm package (`@neralabs/rerail-pack-<id>`) installed into `$RERAIL_HOME/packs`.
- **By its publisher** (`By @acme`): from a third party's pack (`rerail-pack-<id>` or `@<scope>/rerail-pack-<id>`), not reviewed by Rerail.
- **Created by you**: stored once per machine in the library.

`rerail rules export` writes the library and all repository configuration to `$RERAIL_HOME/rules.json` (or `--output <file>`); `rerail rules import <file> [--approve]` loads it back.

<details>
<summary>Pack rules, edited copies and backups</summary>

A third party's pack installs into `$RERAIL_HOME/packs` too. Pack rules are read from the installed packs, never copied into the database. They run only where a repository adds the rule or a pack that holds it.

Editing a By Rerail rule stores a copy in the library, added where the original was. The original switches off there once the copy is approved. From a terminal or any coding agent, `rerail rules show <id>` prints the rule as a draft, and `rerail rules add` of a draft that keeps its id stores the copy ([Testing and adding a rule draft](#testing-and-adding-a-rule-draft)).

`rerail update` first copies the whole database to `$RERAIL_HOME/rerail.sqlite.pre-update`. `rerail reset` never touches rules.

</details>

### Packs

A **rule pack** (`packs/<id>/`, [pack format](pack-format.md)) holds rule documents in `rules/` and a `pack.json`, but no code. Set-up recommends the packs that fit; `rerail packs list`, `add`, `remove` and `recommend` manage them afterwards. See [Rule packs](rule-packs.md).

<details>
<summary>How matching works, and your own packs</summary>

`pack.json` lists each rule's document, title and group, and a match definition saying which repositories the pack fits: file globs, or dependencies in `package.json`, `requirements*.txt`, `pyproject.toml` or `composer.json`.

Set-up fetches every Rerail pack from npm and matches each against the repository locally, with no model. It installs only the packs you confirm, at the versions shown.

For your own conventions, name a pack's `package.json` `rerail-pack-<id>` or `@<scope>/rerail-pack-<id>` and install it with `rerail packs add <folder>`. Rerail's packs use the `@neralabs` scope and win any pack id or rule id clash.

</details>

### Rule documents

A code rule is a `rerail.policy/v1` document: a path `scope` plus regex, AST or relational clauses, each with the violation it reports. The [code-rule DSL reference](code-rule-dsl-reference.html) documents every construct; [Tool-call rules](tool-call-rules.md) covers tool calls.

A conversation rule file:

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

<details>
<summary>Field values</summary>

| Field | Values |
| --- | --- |
| `match.operator` | `gte` (semantic similarity), `lte` (dissimilarity) |
| `match.threshold` | `-1` through `1`, inclusive |
| `sample.source` (event source) | `assistant-text`, `user-input`, `tool:<tool-name>` |
| `sample.boundary` (inspection boundary) | `sentence`, `message`, `turn`, `tool-call` |
| `repeat` | `once`, `always`, `{ "mode": "after-gap", "gapTurns": 3 }` |

A turn, for `sample.boundary: "turn"` and `gapTurns`, is counted at each turn end the coding agent reports. In Claude Code and Codex that is each `Stop`, including each one after a Stop hook kept the turn going; in OMP and Pi it is each model call.

</details>

Every rule, conversation or code, needs a `mode`:

- **`log`** records a match for review; no guidance.
- **`steer`** sends the rule's guidance to the agent.
- **`block`** ([internal testing](../README.md#internal-testing)) steers until block enforcement lands.

<details>
<summary>How Block reports that it steers</summary>

A `block` rule is stored and reported as `block` but delivered exactly like `steer`. Whatever puts a rule on `block` says so. `rerail rules add`, `rerail rules import`, `rerail rules list` and `rerail rules approve` print that Block does not block in this release. `library_rules_upsert`, `repository_settings_set` and `repository_attachments_set` return a `note`. The dashboard shows `Block (steers for now)`.

</details>

Start small, from real incidents, repeated review feedback and key architectural decisions. A rule should give a specific action or invariant, not generic advice.

Good:

> This touches persisted session events. Preserve replay idempotency and verify restoration from an interrupted turn.

Weak:

> Remember to consider edge cases and write tests.

## Rerail database

One SQLite file per Rerail home holds all rule configuration, your rules and the machine's execution history.

The file is `$RERAIL_HOME/rerail.sqlite` (`~/.rerail/rerail.sqlite` when `RERAIL_HOME` is unset). Its main tables:

- **Rule configuration**: `repositories`, `checkouts`, `rule_attachments`, `rule_settings`, `library_rules` and bookkeeping tables ([Rule scoping](rule-scoping.md)).
- **`sessions`**: one row per harness session.
- **`events`**: the append-only transcript.
- **`work_links`**: references from a session to branches, pull requests or issues.
- **`steer_feedback`**: agents' reasons for not following steers ([Steer feedback](#steer-feedback)).
- **`audits`**: every rules run over a session, live or re-run.

<details>
<summary>Schema, access and columns</summary>

Harness adapters, runtimes and the daemon open the file from Node via `drizzle-orm/better-sqlite3`. The OMP extension runs in OMP's Bun-based host and never opens SQLite; it uses the daemon's native-session IPC.

The schema is [Drizzle](https://orm.drizzle.team) tables in `src/engine/storage/schema.ts`. `npm run generate:schema` renders it to the committed `src/engine/storage/schema-sql.generated.ts` (`RERAIL_SCHEMA_STATEMENTS`). `openRerailDatabase` applies it to a fresh file; `test/engine/catalog/rule-store.test.ts` checks it for drift.

- `sessions`: `host`, `cwd`, `repository_id`, `started_at`, `ended_at`. `repository_id`, the checkout's repository key, attributes history to a repository.
- `events`: `kind`, `observed_at`, `payload_json`, or a `payload_v8` blob when not JSON-representable.
- `work_links`: `kind` (such as `git-branch`, `github-pr`, `linear-issue`), `reference`, optional `url`. No foreign key, so links can precede the session row.
- `steer_feedback`: local only; no foreign key.
- `audits`: scope (`repository`, `touched-files`, `ai-output`), independent run mode (`steer`, `log`), session, paths, source revision, timestamps, violations and execution failures. Terminal records are immutable. Closing or reopening a session interrupts unfinished audits, keeping partial results.

</details>

### History across repositories

The daemon serves every repository on the machine; a coding agent on `/mcp` sees only its own repository ([Run](#run)).

<details>
<summary>Filters and code-rule paths</summary>

Listings (sessions, audits, violations, repository audits, rule activity) take an optional `repository` key or alias, else cover everything. A session id is found wherever it was recorded.

Code-rule path filters (`scope.include`, `scope.exclude`; exclusions win) apply to all three audit scopes in either run mode. Paths are relative to the session's `cwd`.

</details>

### Reset, scrub and seed

- **`rerail reset`** deletes the current repository's history across all its worktrees. Rules and settings stay.
- **`rerail scrub`** redacts secrets earlier versions stored, with the scrubber all writes now use (`src/engine/secret-scrub.ts`).
- **`rerail seed`** idempotently adds mock data for MCP and UI work.

<details>
<summary>What each command touches</summary>

`rerail reset` deletes, in one transaction, ended sessions and all events, work links, steer feedback, audits, audit requests and repository audits. Rules, other repositories, audit settings and presentation preferences stay. The file is never unlinked, so open handles are unaffected. A running session keeps its row, minus earlier events, and keeps recording.

`rerail scrub` covers:

- **Database**: event payloads, audit findings (message, captured variables, fix text) and audit errors, in batches, then `VACUUM` in place as `rerail reset` does.
- **Ledgers**: each checkout's `.rerail/sessions/*/events.jsonl` (linked folders skipped). Sample text and delivery content are scrubbed; a secret-holding preimage becomes its SHA-256 and size.
- **Held steers**: `.rerail/sessions/<host>-pending/*.json`, under the hook's session lock.
- **`daemon.log`**.

Ledgers and `daemon.log` are rewritten in place, never shrunk or written past the end last read, so concurrent appends survive. Shorter text is space-padded. Longer text first reserves room with a `{"rerail-scrub-reserve":…}` line, removed by the next pass. Violation identity (clause, path, span, code) is unchanged.

A failing step (a busy lock or full disk, a ledger written mid-line, an unreleased lock) is reported, the rest run, and the exit code is 1. `rerail reset` also exits 1, history already deleted, if its rewrite fails or another reader holds the WAL. Re-run either to finish; re-running a complete scrub changes nothing.

`rerail seed` adds three code rules covering `forbid` with regex, ast and llm clauses, and two conversation rules (one off). They go into the library and the current repository (set up if needed). It also adds a session with events and two work links, attributed to that repository.

</details>

### Steer feedback

An agent that won't follow a steer tells the user why in one line and records it with `rerail feedback`.

```sh
rerail feedback <rule-id> <reason>       # keep the note on this computer
rerail feedback list [--rule <rule-id>]  # print the notes, oldest first, to attach to an issue
```

Rerail sends notes nowhere. To share them with Nera, attach `rerail feedback list --rule <rule-id>` output to a [Rule misfire](https://github.com/neralabs/rerail/issues/new?template=2-rule-misfire.yml) issue. Read the reasons first: each is the agent's own sentence about its work.

<details>
<summary>What is recorded and printed</summary>

A steer asks the coding agent to fix the problem, or tell the user in one line why not and record it: `rerail feedback <rule-id> '<that line>'`. A line containing `'` goes on stdin: `rerail feedback <rule-id> - <<'LINE'`. The id appears in the command only if a shell reads it as one word.

Notes go to `steer_feedback` with the rule's pack and version, repository, coding agent, last-steered session and Rerail version, for tuning rules that get pushback. Ids or reasons with control or invisible characters (bidi override, zero-width space) are refused.

Each listed note shows time, rule id, `<pack>@<version>` (`own rule` if none), coding agent, Rerail version and reason; never session or repository.

</details>

## Install

This section is the install guide, for you or for the coding agent you ask to install Rerail.

In the repository you want Rerail in, run:

```sh
npx @neralabs/rerail setup
```

From a Rerail checkout, `just rerail` does the same. For another repository, answer "Scan a different repo…" or pass `--repo ~/code/your-repo`.

- **What it does.** Installs into `~/.rerail` without admin rights, proposes rule packs, and on your yes starts Rerail and connects your coding agents. Rerail starts in your next new session.
- **In a terminal**: ↑/↓ and Enter, or type the number.
- **From a coding agent**: give it this section. It runs `npx @neralabs/rerail setup --relay` and asks you each question.
- **If your coding agent can't install it, run the same line in a terminal.**
- **If something goes wrong**, see [Troubleshooting](troubleshooting.md) and [Supported and not supported](installation.md#supported-and-not-supported).

<details>
<summary>Set-up choices and relay mode</summary>

Set-up turns nothing on by itself. It asks before reading the repository and gives a reason for each pack. On your yes it starts Rerail in every checkout you haven't stopped on its own. It connects the coding agents it finds (Claude Code, Cursor, Codex, OMP, Pi), or those you name, and ends with a short summary and a dashboard link.

- **Before the scan**: this repo, another, or pick packs yourself (no code read; none is fine).
- **After the scan**: start, change packs, or not now (nothing installed or turned on).
- **Outside a repository**: it asks which folder; or skip and type `/rerail-start` later in a session in any repository.

With `--relay`, set-up prints each question for the coding agent to ask you verbatim, with the command per answer (`rerail setup --relay --answer … --repo … --agents …`), even where the agent has a terminal. The agent waits for your reply and runs the command for your answer. Without `--relay`, set-up does this whenever there is no terminal.

[Supported and not supported](installation.md#supported-and-not-supported) lists the operating systems, Node versions, shells and coding agents Rerail runs with, and those it doesn't.

</details>

Prerequisites (set-up checks them before changing anything, and says what to do):

- Node.js 24 or newer.
- Git on your `PATH`.
- macOS (arm64, x64), Linux with glibc 2.28 or newer (arm64, x64), or Windows MSVC (arm64, x64). Not Alpine or other musl Linux.
- [npmjs](https://www.npmjs.com/), Rerail's only registry; no npm account needed.

<details>
<summary>Why these prerequisites</summary>

- Rerail reads every repository through Git. It skips a `git` inside the repository or `node_modules`, so a repository's `bin/` can't shadow it. With none on `PATH`, it falls back to the usual install locations.
- Linux needs glibc 2.28 because Node 24 does. The package carries prebuilt native kernels for every supported target; nothing is compiled on your machine.
- The package is public, so npm needs no login.

</details>

### What set-up installs

`setup` installs under `~/.rerail/versions/<version>`, points `~/.rerail/current` and the launcher `~/.rerail/bin/rerail` at it, and adds `~/.rerail/bin` to `PATH`.

- `rerail setup [--answer <answer>] [--repo <dir>] [--agents omp,pi,claude-code,codex,cursor] [--port <n>] [--json] [--relay]` takes the answers as arguments.
- `rerail install --agents <list> [--dry-run]` only connects coding agents.
- `rerail install --remove` removes Rerail, keeping `rerail.sqlite` in a dated backup.

<details>
<summary>Launcher, PATH and removal details</summary>

On Windows the launcher is `rerail.cmd`, plus `rerail` for Git Bash. The `PATH` entry is a marked block in your shell startup file, or the Windows user `Path`; `RERAIL_MODIFY_PATH=0` leaves `PATH` to you. `rerail update` adds it where an older set-up didn't. `rerail update` shells out to `npm` with your `~/.npmrc`.

`rerail install --remove` disconnects every coding agent, takes Rerail off `PATH` and stops the daemon. It moves `rerail.sqlite` to a dated backup, so rules and history are kept. Then it deletes `current`, the launcher, every version, the rule packs and `~/.cache/rerail`, naming each.

</details>

[Installing Rerail](installation.md) is the short guide; [How set-up works](how-setup-works.md) details each step, removal and the managed layout.

### Commands in each coding agent

Every coding agent gets the same commands ([command reference](#command-reference)). Restart the coding agent after installing or rebuilding.

- **OMP and Pi** register them natively.
- **Claude Code and Cursor** get user-level command files in `~/.claude/commands/` and `~/.cursor/commands/`.
- **Codex** gets one skill per command in `~/.codex/skills/<name>/SKILL.md`, invoked as `$rerail`, `$rerail-manage`, `$rerail-start`, and so on.

`/rerail-start` covers the current checkout and the worktrees that follow it; `/rerail-stop` stops them and detaches the runtime immediately.

Settings, audits, status, version, update checks and reset are `rerail` CLI commands. See [Scan and settings controls](#scan-and-settings-controls) for targeting and coding-agent differences.

<details>
<summary>Registration details</summary>

From the main checkout, `/rerail-start` covers every linked worktree without its own setting. Claude Code and Cursor command files come from `rerail install` or `rerail setup` and are removed by `rerail install --remove`. The CLI commands are `rerail settings`, `rerail scan`, `rerail status`, `rerail version`, `rerail update check` and `rerail reset`.

Registration removes Rerail's own files under retired names (`/rerail-session`; `/rerail-ui` and `/rerail-rage-quit`, now `/rerail-manage` and `/rerail-uninstall`). `rerail update` re-registers connected agents (`rerail install --refresh`); a coding agent never connected stays untouched. Generated files are replaced without a `.bak`; a same-named file Rerail didn't write is backed up first.

</details>

### Native targets and host entrypoints

Native targets: macOS arm64 and x64; Linux glibc arm64 and x64, built against glibc 2.17; Windows arm64 and x64 (MSVC).

OMP and Pi run Rerail inside their extension host; Claude Code, Codex and Cursor hooks send events to the daemon.

<details>
<summary>Entrypoints and event paths</summary>

- **OMP**: `dist/hosts/omp/default-extension.js`, in the package's `omp` manifest.
- **Pi**: `dist/hosts/pi/pi-extension.js`, in the `pi` manifest; loadable with `pi -e`.
- **Persistent event stream**: `dist/harness/event-stream.js` or `rerail events` ([internal testing](../README.md#internal-testing)).
- **Claude Code and Codex**: `createClaudeEventHandler(...)` and `createCodexEventHandler(...)` for harness-owned hook registration.
- **Cursor**: `.cursor-plugin/plugin.json`, backed by `dist/hosts/cursor/cursor-hook.js`.

Lifecycle ingestion is not an MCP tool surface. OMP and Pi run a session-scoped `RerailRuntime` in their extension host. Claude Code, Codex and Cursor hooks send normalized events to the daemon's resident kernel over an owner-only Unix socket or local Windows named pipe, so policy state outlives each hook process. Persistent integrations can use the newline-delimited `RERAIL-EVENTS/1` stream. All share one event runtime, cosine registry and executable-policy bridge.

</details>

## Activate

Activation binds a machine to a signed customer profile; it is in [internal testing](../README.md#internal-testing).

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

The trust-root file must arrive through an authenticated channel independent of the profile. Device private keys and returned credentials go to macOS Keychain, Linux Secret Service or Windows Credential Manager. The signed profile selects the customer control plane and limits automatic updates to its authorized npm dist-tag.

## Update

Updates are explicit: Rerail never checks a registry or installs on its own.

```sh
rerail version            # the installed version
rerail update status      # active and previous versions, channel, last check
rerail update check       # resolve the channel without installing
rerail update             # install and activate a newer version
```

`rerail update` backs up the database, installs and validates the new version, activates it, and re-registers your connected coding agents.

<details>
<summary>Update steps and failures</summary>

`rerail events`, hooks and the daemon run only the active version. `rerail update`:

1. Copies the database to `$RERAIL_HOME/rerail.sqlite.pre-update` for manual restore ([Database compatibility](how-setup-works.md#database-compatibility)).
2. Installs the configured npm dist-tag's version under `~/.rerail/versions`.
3. Validates its native runtime and event stream.
4. Activates it atomically for new harness sessions; `rerail update status` names the replaced version.
5. Updated or not, re-registers every connected coding agent (`rerail install --refresh`, which also runs on its own), adding new commands, skills and hooks and removing retired ones. A coding agent never connected is left alone.

An agent that can't be re-registered is named with the reason, the rest are refreshed, and the update exits 1 with the new version active. One whose configuration can't be read is named and skipped. Coming from a version without step 5, run `rerail install --refresh` once.

</details>

## Run

`rerail daemon start` runs the rules kernel, transcript database and browser view as one background daemon.

```sh
rerail daemon start            # background daemon on the port it last ran on, else http://127.0.0.1:4737/ (a free port when those are held), opens nothing
rerail daemon start --open     # same, then opens the browser view
rerail daemon start --host 0.0.0.0 --allow-remote --public-host rerail.example --tls-cert cert.pem --tls-key key.pem   # bind off loopback over https (see below)
rerail start                   # start Rerail in this checkout and the checkouts inside it (--repository: every checkout); in a repository that is not set up, first runs set-up's questions there
rerail stop [<host> [<session-id>]]   # stop Rerail in this checkout and the checkouts inside it (--repository: every checkout); the shared daemon keeps running for other checkouts and coding agents, and the reply names rerail daemon stop. A coding agent's host sets the reply's command sigil, and its session (rerail stop codex: CODEX_THREAD_ID) makes the reply name the rules that steered it
rerail state                   # prints started, stopped, or not-set-up for this checkout
rerail status                  # whether Rerail is on here, then the daemon, its engine and its model (Model: ok, or the problem and file, e.g. checksum mismatch (model.safetensors)); exit 3 when the daemon is not running, 4 when it does not answer, 5 when its engine or model is not ok
rerail manage [--print]        # prints the dashboard's Overview URL and opens it; --print only prints
rerail session [<host> <session-id>] [--open]   # this session's status line and page URL; --open opens it
rerail daemon stop             # shut the daemon down for every checkout and coding agent; graceful stop where supported; forced termination after 10 s, and an error if the process outlives that by 5 s. A coding agent's session in a checkout where Rerail is started starts it again with its next event, unless RERAIL_AUTOSTART=0
```

<details>
<summary>Native binding check before launch</summary>

Before spawning, `rerail daemon start` probes the `better-sqlite3` binding under the daemon's Node. It rebuilds the binding in place if a Node upgrade changed the ABI. If it still fails, the launch stops with the reason rather than crashing into `~/.rerail/daemon.log`. Autostart runs the same check, and `rerail update` applies it to an installed version that fails validation.

</details>

### What the daemon serves

The daemon owns the resident kernel and exposes the database over HTTP, OpenAPI and MCP.

- **`GET /healthz`**: version, health, and engine and model status.
- **`GET /openapi.json`**: OpenAPI 3.1 for `/api`.
- **`POST /mcp`**: MCP Streamable HTTP, each `/api` operation a same-named tool.
- **`/api/*`**: the operations below.
- **Anything else**: the bundled UI.

<details>
<summary>Endpoint details and degraded modes</summary>

The daemon writes rules and work links; the shared runtime records harness events.

- `/healthz` reports version, pid, database path and presence, kernel status, `installedAt`, and `model`: bundled model files checked once per process against `model/manifest.json`, `ok` or the problem and file. `healthy` means engine and model are ok; `status` only means the daemon answers.
- `/mcp` is stateless and JSON-response. Non-POST gets `405 Allow: POST`; there is no SSE listener or transport session.
- UI client-side routes fall back to `index.html`.

Without the native addon the daemon still runs: `status`, `/healthz` and the UI header report the kernel unavailable, and transcripts stay readable. Model files can fail `model/manifest.json`; a clone without Git LFS has a pointer for `model.safetensors`. Then `rerail status` prints `Model: checksum mismatch (model.safetensors)` and exits 5, and `/healthz` reports it. The dashboard header turns red naming the file, and session attaches fail until the files are fetched and the daemon restarted.

</details>

### API operations

Every operation is both an HTTP route under `/api` and an MCP tool of the same name. Each is defined once (id, route, zod schemas, handler) in `src/api/operations.ts` or `src/api/repository-operations.ts`, so a new operation needs no other wiring.

<details>
<summary>All operations, wiring and error codes</summary>

| Operation (HTTP and MCP tool) | Route | Purpose |
| --- | --- | --- |
| `repositories_list` | `GET /api/repositories` | Known repositories: key, name, aliases, set up, started, checkout count. |
| `repository_context_get` | `GET /api/repository?repository=&checkout=` | A repository's clones and checkouts (own and effective state), and the caller's checkout and folder. |
| `repository_rules_get` | `GET /api/repository/rules?repository=&checkout=` | Rules per attachment folder with on, mode, suppressions, config, what decided each, `addedAt`, and `updatedAt` for your rules; unresolvable ones marked unavailable. |
| `repository_settings_set` | `PUT /api/repository/settings` | On/off, mode, `suppressions` per repository or checkout (`owner`) and folder; By Rerail code rule `config` (whole repository). Null clears. |
| `repository_follow_gitignore_set` | `PUT /api/repository/follow-gitignore` | Follow `.gitignore` (default on): ignored files aren't audited (shell writes, direct edits, repository audits); tracked files are. A tool edit it leaves unchecked, and a change that makes Git's ignore rules ignore more, are recorded on the session (`sessions_get` `gaps`; [what is and isn't](in-session-surfaces.md#files-git-ignores)). New sessions only; summary field `followGitignore`. Agents may only turn it on, before set-up. |
| `repository_attachments_set` | `PUT /api/repository/attachments` | Add or remove packs, By Rerail rules and library rules per folder, with optional settings. The first addition (or `configure: true`) sets the repository up. |
| `repository_start` | `POST /api/repository/start` | Start; `scope` is `checkout` (and checkouts inside) or `repository`. |
| `repository_stop` | `POST /api/repository/stop` | Stop, same scopes; rules and history kept. |
| `repository_link` | `POST /api/repository/aliases` | Resolve another remote address here. People only. |
| `repository_reset` | `POST /api/repository/reset` | Clear history like `rerail reset`. Ended sessions go; running ones keep recording; rules and settings stay. People only. |
| `rules_activity` | `GET /api/rules/activity?since=&repository=` | Per rule since a time: violations (log-only count), sessions, last sighting, `audits`, steers; removed rules with violations as `removed: true`. `sessions: { started, withViolations }`. |
| `rule_packs_list` | `GET /api/rule-packs?checkout=` | Packs with npm package, publisher (`rerail`, or a third party's scope or package name) and rules (kind, title, group, mode, on/off, examples). With a checkout (MCP: the caller's), also its `.rerail/rules` as pack and publisher `worktree`, skipped files in `skipped`. Read-only. |
| `rule_packs_recommend` | `GET /api/rule-packs/recommendations?repository=&checkout=` | Packs fitting a checkout: rule count, reason, folder. Local; changes nothing. |
| `coding_agents_list` | `GET /api/coding-agents` | Per coding agent: `found`, `connected`, `problem`, and whether its tools can read Rerail's data (`rerailData`: `access` is `kept-out`, `file-tools` or `open`, with `why`). Dry run; shown in Settings and `rerail status`. |
| `rerail_rules_list` | `GET /api/rerail-rules?kind=` | By Rerail rules with their pack. |
| `library_rules_list` | `GET /api/library-rules?kind=` | Rules created by you. |
| `library_rules_get` | `GET /api/library-rules/{id}` | One rule created by you. |
| `library_rules_upsert` | `PUT /api/library-rules/{id}` | Create or replace your rule (`attach: true` adds it to the target); a By Rerail id stores an edited copy. |
| `library_rules_remove` | `DELETE /api/library-rules/{id}` | Delete your rule, its attachments and settings. |
| `sessions_list` | `GET /api/sessions?since=&limit=&cursor=` | Sessions newest first with counts, `steeredViolations`, `lastEventAt`; `total`, `nextCursor`. |
| `sessions_get` | `GET /api/sessions/{sessionId}` | Summary with `gaps` over the whole transcript: undelivered events, boundaries answered before checks ran, tool edits left unchecked because Git ignores the file (`ignoredEdits`), changes seen that make Git's ignore rules ignore more (`ignoreRuleChanges`). |
| `session_events` | `GET /api/sessions/{sessionId}/events?after=&before=&limit=` | Ordered events, at most 500 per page, forward from `after` or the page before `before`; `nextAfter` continues. Agents get some payloads withheld ([details](#what-a-coding-agent-may-read)). |
| `work_links_list` | `GET /api/sessions/{sessionId}/work-links` | A session's external references. |
| `work_links_add` | `PUT /api/sessions/{sessionId}/work-links` | `{ kind, reference, url? }`; re-linking updates the url. |
| `audit_settings_get` | `GET /api/sessions/{sessionId}/audit-settings` | Defaults (`scope`, `mode`, experimental `steerStrategy`) and the first `repositoryAudit` for the session's canonical workspace, with its `repositoryAuditRecord`. |
| `audit_settings_update` | `PATCH /api/sessions/{sessionId}/audit-settings` | People only (`rerail settings`). Set defaults, which outlast the session: `mode` `steer`/`log`; `steerStrategy` `message`/`rewind`/`subagent`. |
| `audit_requests_create` | `POST /api/sessions/{sessionId}/audit-requests` | Internal testing. Queue an audit for a session. |
| `audit_requests_list` | `GET /api/sessions/{sessionId}/audit-requests` | Internal testing. `{ auditRequests }` with their `auditIds`. |
| `audits_list` | `GET /api/audits?auditId=&sessionId=&status=&scope=&mode=` | `{ audits, total }` with `violations` and failures; `mode` is run mode. |
| `violations_list` | `GET /api/violations?since=&ruleId=&sessionId=&limit=&cursor=` | Violations newest first with `ruleId`, `clauseId`, session, `host`, `auditId`, and `logged` (the agent wasn't told: the audit or the rule was in Log mode); `total`, `nextCursor`. |
| `repository_audits_list` | `GET /api/repository-audits` | Internal testing. Each repository's `repositoryAudit` and latest `audit` summary with `violationCount`. |
| `repository_audit_entries_list` | `GET /api/repository-audits/{auditId}/entries?kind=&mode=&code=&path=&q=` | Internal testing. Paged entries; `kind` `violation`/`failure`, `mode` the rule's mode. |

`src/api/http.ts` mounts the operations with Hono and emits OpenAPI; `src/api/mcp.ts` registers them as MCP tools.

Validation failures return `400 { error: "invalid-request", issues }`. Handler errors carry a status and a code: `rule-not-found`, `rule-id-mismatch`, `rule-conflict`, `repository-audit-not-found`, `target-required`, `target-mismatch`, `agent-not-permitted`, `invalid-cursor`.

</details>

### MCP bridge

`rerail install` registers a `rerail` MCP server with OMP that relays requests to the running daemon. OMP lists it under `/mcp`; run `/mcp reload` if the daemon started after the session. Other harnesses can run the same command or read the URL from `daemon.json`.

<details>
<summary>How the bridge finds the daemon</summary>

`rerail install` (and `rerail setup`) adds a `rerail` entry to `~/.omp/agent/mcp.json`, beside the extension symlink. It is a stdio server running the installed `rerail mcp-bridge` (`{ "command": "<node>", "args": ["<install>/dist/cli/cli.js", "mcp-bridge"] }`). `--remove` deletes only that entry.

The bridge relays to the daemon recorded in `daemon.json`, following port changes and restarts, so the entry never needs editing. It first waits for that daemon's `/healthz` to answer with the recorded pid. A stale file therefore yields "Rerail daemon is not running" rather than reaching whatever holds the port.

After that check a request costs one file read, and the bridge re-checks only on connection failure. Suppose a checked daemon is killed with SIGKILL, its pid is reused, and another daemon takes its port before the next request: that request reaches the other daemon.

</details>

### Targets and what a coding agent may change

`/api` callers are people; `/mcp` callers are coding agents. This scopes agent tools; it is not authentication, since any local process can call `/api`.

An agent reaches only its checkout's repository and checkouts nested in its own, and may change little there. Anything else is `403 agent-not-permitted` with the dashboard URL to do it there: `<origin>/?repository=<key>&checkout=<path>#rules`. See [Rule scoping](rule-scoping.md).

<details>
<summary>Targets and exactly what an agent may change</summary>

Repository and rule operations take a target in the query or body: `repository` (a key such as `remote:github.com/acme/app` or `clone:<id>`, or an alias), `checkout` (an absolute path; wins over `repository`) and `folder`. An MCP call with neither uses the caller's checkout, from the `x-rerail-caller-cwd` header the bridge sends: the URI-encoded real path of the session's starting folder.

`repositories_list` shows an agent only its own repository. Naming another, or calling without a caller folder, is `403 agent-not-permitted`. Within its reach, an agent may:

- start, stop and change settings in its checkout and those inside it;
- change attachments and settings only before set-up, or to add a rule it creates in the same call, or its copy of a By Rerail rule;
- edit or delete your rule only where no other repository runs it;
- turn Follow `.gitignore` on, never off, set up or not: reading ignored files is the person's choice.

Status line preferences and ongoing audit settings belong to the person.

</details>

### What a coding agent may read

An agent reads only its own repository's history, since transcripts hold prompts and tool output and violations hold source. `session_events` gives it prompts, assistant text, lifecycle, steers and Rerail's bookkeeping; payloads it may not see become `{ withheld, reason }`.

<details>
<summary>How other repositories are hidden, and what is withheld</summary>

- `sessions_list`, `violations_list`, `audits_list`, `repository_audits_list` and `rules_activity` cover its repository. Naming another `repository` is `403 agent-not-permitted`, as is an MCP call without `x-rerail-caller-cwd`.
- Another repository's session or audit looks missing. `sessions_get`, `session_events`, `work_links_list`, `work_links_add` and the audit settings and request operations return `404 session-not-found`. `repository_audit_entries_list` returns `404 repository-audit-not-found`. `presentation_get` returns no `session`.

`session_events` withholds:

- every `result`'s `output`, since it may be a file's contents;
- a `call`'s `input` naming a hidden or secret-like path (`cat .env`, `.ssh/id_rsa`, `cat {.env,.npmrc}`, a Kubernetes `..data/`; see `src/engine/path-boundary.ts`), or a session-folder path linking to one (`docs/a.md` linking to `.env`), though not one merely inside a hidden folder like `node_modules/.pnpm/`. A call's first 64 tokens are resolved, the rest judged by name;
- a `call`'s `input` too deep or large to read whole;
- an event kind not listed in `src/engine/transcript-boundary.ts`, whole, as `{ withheld: "event payload", reason }`.

The check reads names, not a shell, so paths built at run time are missed. `rerail rules evidence` names such calls by tool only, and lists no Git-ignored standards file.

Violations in secret-named files (`secrets.yaml`, `credentials.json`, `token.txt`, `gcp-key.json`, `kubeconfig`, `prod.sops.yaml`, anything under `.aws/`; `SENSITIVE_FILE_NAMES` and `SENSITIVE_FOLDER_NAMES`; hidden alone doesn't count) reach an agent by rule and place only. The steer, `violations_list`, `audits_list` and `repository_audit_entries_list` withhold the message (which metavariables may fill with file text), `variables` and fix, and the steer says so. The steer also judges a file by where it links. Audits still run, and people see full findings via `/api` and the dashboard.

</details>

### Network binding and TLS

The daemon binds `127.0.0.1` by default (`--port`, `--host` override). Any `--host` other than `127.0.0.1`, `localhost` or `::1` needs `--allow-remote`, since the whole network could then reach it.

A daemon reachable from other machines (off loopback, or answering a `--public-host`) serves https with `--tls-cert <pem>` and `--tls-key <pem>`. A self-signed certificate works: `openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 365 -subj /CN=rerail -keyout key.pem -out cert.pem`.

<details>
<summary>Certificate rules and pinning</summary>

`--tls-cert` takes the certificate, or a chain with the server's first. Both files are read once at start, and also work on loopback. A key file with any group or other permission bit (such as `0644`) is refused; `chmod 600` it.

Without TLS such a daemon won't start, since the access capability would cross the network in clear. `--insecure-http` overrides this for a trusted network: a container port published only on host loopback, or a local TLS-terminating proxy.

The daemon records an `https://` URL and its SHA-256 fingerprint (`tlsFingerprint256`) in `daemon.json` and prints the fingerprint. Rerail's clients (CLI, hooks, MCP bridge, OMP, Pi) accept only that exact certificate, whatever its issuer or names. Browsers warn; compare fingerprints.

</details>

### Access capability

Every request needs the daemon's access capability in the `x-rerail-access` header, or gets `401 { "error": "access-required" }`. The dashboard link carries it; `rerail manage` prints and opens that link.

<details>
<summary>Scope, storage and lifetime</summary>

The header covers routes, `/mcp`, `/native/*`, `/healthz` and `/openapi.json`. Only the page's static files (`/`, `/index.html`, its `.js`, `.css` and other files outside `/api/`) load without it, since npm ships them to anyone. The page then sends the capability on every request. `Host`, `Origin` and loopback are no boundary: another local user, or a sandboxed process allowed localhost, reaches `127.0.0.1:4737` as you do.

- **Value**: 256 random bits, stored as `accessToken` in `daemon.json` (mode `0600`) for Rerail's processes (CLI, hooks, MCP bridge, OMP, Pi).
- **Lifetime**: a loopback daemon keeps it in `access-token` in the Rerail home (mode `0600`) across restarts; delete the file to rotate. With `--allow-remote` or `--public-host`, a new one is minted every start.
- **Browser**: it arrives in the dashboard link, `<url>?rerail-access=<capability>`. The page stores it per origin (scheme, host and port), strips it from the address, and is served with `Referrer-Policy: no-referrer`. A header, not a cookie: cookies reach every port of a host, while a custom header can't cross origins without a CORS preflight, which the daemon never answers.

</details>

<details>
<summary>Where the link is printed</summary>

`rerail daemon start` prints the link at a terminal (`--open` opens it), but not when piped, as for a coding agent. For a daemon on every interface it says `127.0.0.1`; adjust the host. `rerail manage` prints the plain URL when piped. `rerail session --open`, set-up and `/rerail-manage` add the capability from `daemon.json` when opening your browser. Links for coding agents (session-start note, a steer's `why?`) never carry it; a browser without it is told to run `rerail manage`.

</details>

An agent with a shell runs as you and can read `daemon.json`: the capability separates you from other users and sandboxes, not from that agent; only the operating system can. If the daemon rejects the recorded capability (edited, or another daemon's), `rerail status`, `rerail daemon start` and the MCP bridge say so; run `rerail daemon stop`, then `rerail daemon start`.

### Host and origin checks

Every request's `Host` header must name loopback, the `--host` address or a `--public-host` name, which blocks DNS rebinding.

<details>
<summary>Accepted hosts and response headers</summary>

Loopback alone doesn't authorize writes, because DNS rebinding lets a page on another site resolve its name to `127.0.0.1`. `Host` must name:

- `127.0.0.1`, `localhost` or `[::1]` on any port (port mappings and reverse proxies present ports the daemon never bound);
- the literal `--host` address (IP literals can't arrive by rebinding, and the daemon's state URL dials it);
- a hostname from `--public-host <name,...>`.

Anything else, including `/native/*`, `/mcp`, `/healthz` and the page, gets `421 { "error": "host-not-allowed" }` before routing. An `/api` write with an `Origin` must come from one of those hostnames on the addressed port, or gets `403 api-origin-forbidden`; Rerail's processes send no `Origin`. The page is served with `X-Content-Type-Options: nosniff`, its HTML with `Content-Security-Policy: frame-ancestors 'none'` and `X-Frame-Options: DENY`, so no site can frame it.

</details>

### Daemon state

`~/.rerail/daemon.json` (or `$RERAIL_HOME/daemon.json`) holds pid, port and database path; output goes to `daemon.log` beside it. `RERAIL_AUTOSTART=0` stops automatic daemon creation, so command hooks need an existing daemon or explicit endpoint.

<details>
<summary>Ports, reuse and the kernel socket</summary>

`daemon-port.json` remembers the last port (or `--port`), reused by a start without `--port` so dashboard links survive restarts. `rerail daemon start` reuses a running daemon and says where; stale state from a crash or reboot is cleared. `stop`, `status` and `manage` run from the launcher you invoked, not the managed version, so they work mid-update.

`daemon.json` also records `kernelSocketPath` (Unix socket or Windows named pipe), which hooks reuse; `RERAIL_ENGINE_SOCKET` selects an explicitly managed endpoint. Events there need `x-rerail-access`. For a `RERAIL_ENGINE_SOCKET` socket, hooks send the capability from the Rerail home's `daemon.json` if it names the same socket.

</details>

### Session storage

Each session keeps its ledger (events and file preimages) under a storage root, `<workspace>/.rerail/sessions` by default, and loads the bundled embedding model. `RERAIL_STORAGE_ROOT` and `RERAIL_MODEL_PATH` replace them: a relative value resolves against the session's workspace, the folder the coding agent runs in, and an empty one counts as unset. Hooks, `rerail events`, OMP and Pi (in process or through the daemon) and the daemon itself all resolve them this way. Only the default root gets a `.gitignore` of `*`; a custom one may hold your own files, so Rerail leaves it as it is.

### Event ordering and edit capture

Each session has its own serialized lane, and native tool-call IDs pair edits with their results.

<details>
<summary>Ordering, restarts and per-host capture</summary>

- Pairing holds even for concurrent calls to one tool.
- Lanes are keyed by runtime configuration and session ID. One agent's open, audit or delivery never holds up another, and separate databases or workspaces never share a lane, even when session IDs coincide.
- Pending calls and captured preimages survive daemon restarts.
- Turn checkpoints catch filesystem changes from shells or tools without explicit edit paths. They keep source fingerprints, not a copy of the source or a baseline of violations.

OMP captures pre-edit source in its awaited `tool_call` hook and submits post-edit source from `tool_result` for background evaluation. Its asynchronous `tool_execution_start` and `tool_execution_end` notifications aren't execution barriers and aren't used.

Cursor Read and Write hook phases may share a native tool-call ID but keep distinct normalized event identities. If the CLI omits prompt and response hooks, Rerail reads the host's JSONL transcript incrementally with a durable byte cursor; native prompt hooks win when present. Transcript tool-use records never become invented tool events.

</details>

### Scan and settings controls

An audit runs code rules at one of three scopes, in one of two independent run modes. Full-repository scanning (`rerail scan`, explicit audit requests, the automatic first repository audit, repository audit operations) is in [internal testing](../README.md#internal-testing).

#### Scopes and run modes

| Scope | Reportable code |
| --- | --- |
| `repository` | All eligible code, existing violations included; for diagnostics. |
| `touched-files` | Whole touched files, existing violations included. |
| `ai-output` | Added or changed lines, from captured pre-edit source. Untouched regions report nothing. |

- **`steer`** run mode records results and sends violations of `steer` and `block` rules to the agent; `log` rules' violations are only recorded.
- **`log`** run mode records the same with no conversation output.

Nothing is acknowledged, baselined or suppressed as already seen: changing relevant source can raise the same violation again.

#### Steer strategies (experimental)

The workspace's `steerStrategy` ([internal testing](../README.md#internal-testing)) sets how a code-rule steer reaches the agent: `message` (default), `subagent` or `rewind`. It needs `RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1` for the process running Rerail's runtime; otherwise every steer is a `message`.

<details>
<summary>What each strategy does</summary>

The runtime process is the daemon for OMP, Claude Code, Codex and Cursor, and the agent for a Pi session running Rerail in process.

- **`message`** (also how older saved settings read): a separate message.
- **`subagent`**: the same, ending with an instruction to delegate the fix to a subagent (`rerail_fix` in Pi, registered by Rerail; `task` in OMP; `Task` in Claude Code and Cursor; `spawn_agent` in Codex) and stay on task.
- **`rewind`**: if an edit's own check steers before its tool result is answered, every file the call captured returns to its preimage (created files deleted), all or none. The steer goes into that tool result, saying what was restored and that the edit must be redone following the rule. The rollback is audited in `log` mode, so the restored file steers nothing on its own.

`rewind` falls back to `message` when the preimage was unreadable, the call didn't capture the file, or the steer came after the result was answered (a one-shot hook waits a second). Ledger `delivery` entries record each steer's `strategy`, plus a rewound edit's `toolCallId` and `rewound` paths.

</details>

#### Defaults and requested audits

Rerail audits a repository only when asked, and user-requested scans are `log`-only.

<details>
<summary>Runtime defaults and the first repository audit</summary>

`RerailRuntime` accepts ongoing `auditSettings: { scope, mode, steerStrategy }`; `NativeCodeRuleEnforcer` accepts `scope` and `mode`.

With `RERAIL_EXPERIMENTAL_FIRST_REPOSITORY_AUDIT=1` set for the runtime's process, Rerail runs one independent `log`-mode full-repository audit the first time it sees a canonical workspace in local storage. It is never repeated there. User-requested scans cover the whole eligible repository or the tracked, staged and untracked files in the current Git diff.

</details>

#### How AI-output scope finds changed lines

AI-output scope diffs source, not violation IDs: it compares each edit with pre-edit source captured at the host's tool-call boundary. A new file is entirely generated. A file Git wrote is no edit at all, whichever session ran the command, and neither is one changed while none of the session's tool calls ran.

<details>
<summary>Fallbacks when no pre-edit source was captured</summary>

Without a boundary (a shell write, a Codex `apply_patch` its hooks don't show), Rerail uses the file's last-seen text this session (from the most recently changed 64 MB). For a file Git lists as modified or untracked when Rerail first sees it (`git ls-files --modified --others --exclude-standard`), that is the text it held then, up to 8 MB of such files per session. Else it uses the file's Git index or `HEAD` blob. Each counts only if its SHA-256 matches the pre-change file. Blob reads never fetch a partial clone's missing objects.

If nothing matches, the edit reports its whole file, as `touched-files` does, until Rerail has seen the file change in the session. Parsing and cross-file query context may exceed reporting scope. Source, parse and match caches are incremental; repository scans stream bounded batches through a separate native workspace.

</details>

<details>
<summary>What counts as Git's write</summary>

- a file that now holds exactly its `HEAD` blob after anything but a commit moved `HEAD` (a checkout, switch, pull, merge, rebase, reset), or with `HEAD` where it was (`git restore`, `git stash`);
- a file that now holds exactly the blob a branch or remote-tracking branch has at its path (`git show origin/main:path > path`, `git checkout main -- path`, `git restore --source main path`), unless that branch's tip is a commit made since Rerail last looked;
- a file a move removed;
- the files a merge or rebase wrote when it stopped on a conflict (a later edit to one is an edit, even if the file still holds its side of the conflict);
- a new linked worktree's files.

A file written and committed in one command stays an edit, measured from the tree before the commit. One written in the command that moved `HEAD` is measured from what Git wrote. Rerail goes by bytes: any write that leaves a file exactly as a branch has it counts as a restore, and a restore from a tag, a stash or another commit (`git show HEAD~3:path > path`) is an edit.

</details>

<details>
<summary>What counts as the session's own write</summary>

A session's walk finds that a file changed, not who changed it, so it goes by the file's timestamp, and by its folder's for a file removed. An agent writes only through its tool calls, so each session notes when each of its calls ran, whatever the tool, including those answered while its code rules were still opening. A Codex `apply_patch` runs no hook, so a Codex session counts its whole turn, from your prompt (or, after a daemon restart, its next tool call) to the turn's end, as one call. A change stamped while one of them ran is the session's own. Any other is another writer's (another session, your editor, a process a command left running in the background): it steers nobody, and the session's next shell edit of the file is measured from what that writer left; where the session had changed the file, the change is recorded in Log with no generated lines. When two sessions' calls ran at once, both take a change made then, so a Codex session in the middle of a turn takes another session's change too. A call whose result never comes (one you interrupted) counts as running until the turn ends.

</details>

#### Delivery

OMP and Pi deliver steers in the background; polling hook hosts get buffered violations at their next event.

<details>
<summary>Revisions, superseded steers and failures</summary>

Results carry source revisions: empty checkpoints keep buffered violations; relevant source changes invalidate them.

A newer edit drops a steer still queued for its file. In AI-output scope it also reports the lines generated since, as it does for an earlier edit it superseded before evaluation. An unseen violation therefore arrives once if still present, never if fixed. A shown one isn't repeated by edits elsewhere in the file.

Delivery is not an audit: it has no acceptance callback, acknowledgement persistence, retry chain or exactly-once guarantee. An audit returns violations and execution failures, never `unknown`. Unreadable sources, unavailable generated ranges, missing capabilities, exhausted budgets and evaluator errors go to `audits`, never the conversation. `audit_requests_create` requests an audit; `GET /api/audits` pages records.

</details>

#### Command reference

OMP and Pi expose these natively, with slash-completion descriptions; Claude Code and Cursor as user-level prompt commands; Codex as `$rerail-*` skills.

| Command | Interaction |
| --- | --- |
| `/rerail` | Whether Rerail is on here, rule count, steers this session, dashboard link and all commands. |
| `/rerail-manage` | Open the dashboard Overview for this repository and checkout. |
| `/rerail-create` | Create or tune a rule in your words, or from suggestions. See [docs/writing-rules.md](writing-rules.md). |
| `/rerail-start` | Start in this checkout and its worktrees, attached to the session; runs set-up's questions first if needed. |
| `/rerail-stop` | Stop recording, evaluation, steering and daemon autostart here and in its worktrees. |
| `/rerail-update` | Install the newest version. |
| `/rerail-uninstall` | `rerail install --remove`, keeping a backup of rules and history. Asks first. |
| `/rerail-help` | Same as `/rerail`. |
| `/rerail-show-me` | One-minute demo: the agent breaks a demo rule, is steered and fixes it. |
| `/rerail-bug-report` | The agent reads a support bundle to tell an engine bug, a shipped-rule bug or your rule's bug. |

<details>
<summary>Command details</summary>

- **`/rerail`**: rendered by `rerail session`; Cursor gets the repository-level form.
- **`/rerail-manage`**: the same link the footer's rule count carries.
- **`/rerail-create`**: suggestions come from sessions (`rerail rules evidence`) and written standards. Following `skills/rerail-create`, the agent drafts the rule, tests it (`rerail rules test <draft.json>`) and reads it back. On your yes, `rerail rules add <draft.json>` and `rerail rules approve <id>` add it to your library and this repository, setting the repository up with that rule if needed. It also tunes misfiring rules, changes where a rule applies (its scope only, a shipped rule's as your copy, without the full review), and audits a pending rule or pack before asking you to approve it.
- **`/rerail-start`**: OMP and Pi ask set-up's questions via their picker; Claude Code and Cursor as text.
- **`/rerail-stop`**: the shared daemon keeps running; the reply names `rerail daemon stop`, run in a terminal.
- **`/rerail-uninstall`**: OMP and Pi ask via a native selector; Claude Code doesn't pre-approve it.
- **`/rerail-show-me`**: `rerail show-me` adds rule `local:rerail-show-me`, scoped to `rerail-show-me.js`. The agent writes that file breaking the rule, is steered, and fixes it. `rerail show-me --cleanup` removes both (a concurrent demo elsewhere keeps the rule). It needs Rerail on and set up, never starts or sets up Rerail, and touches only what it wrote. The steer stays in History.
- **`/rerail-bug-report`**: following `skills/rerail-bug-report`, the agent reads `rerail support-bundle` output (allowlisted, scrubbed diagnostics). It fixes your rule via `/rerail-create`'s Tune. Otherwise it checks the bundle with you and offers a filled-in issue, or files it if asked. Nothing is sent without you ([what the bundle holds](troubleshooting.md#reporting-a-problem)). Claude Code pre-approves the bundle's fixed invocations, `rerail update check` and `rerail rules test`; `--output`, `rerail rules add` and filing ask first.

</details>

#### Testing and adding a rule draft

A draft is `{ kind, rule, folder?, cases: { caught: [...], passed: [...] } }`. `rerail rules test --help` prints the format; [Writing rules](writing-rules.md#testing-a-draft) explains each field.

- `rerail rules show <rule id> [--repo <dir>]` prints a rule as a draft: one of yours, or a By Rerail rule as this repository runs it (the option values and mode the repository's settings give it, not a checkout's). A note on stderr, so stdout saves as a draft file, says where saving changes it: your rule here and in which other repositories, or that it is not added here; for a By Rerail rule, your copy, the copy this repository already has, or that the rule is not added or is off here (a copy would run on).
- `rerail rules test` validates a draft, runs its cases and a repository pass, and writes nothing.
- `rerail rules add` stores the draft in your library and adds it, on, to this repository (or the draft's `folder`). A draft that keeps a By Rerail rule's id is stored as your copy (`<id>-copy`), added where the original is in this repository; approving the copy turns the original off there ([copy on edit](rule-scoping.md#coding-agents)).
- The rule runs in Log until `rerail rules approve [<rule id | package> …] [--repo <dir>]` approves it as it is now ([approval](rule-scoping.md#approval)).

<details>
<summary>Test output and add details</summary>

- A case some clause couldn't check (say, unparseable) shows `not checked (<reason>)`, never passed.
- The pass lists files the rule couldn't check and counts in-scope binary files, which no rule reads.
- Text output shows the first five matches and files; `--json` shows all. Path and `file`-query matches have no line.
- A bare rule document, such as `.rerail/rules/<name>.json`, gets validation and the repository pass only.

`rerail rules add` sets the repository up with only that rule if needed. A rule already added here stays where it is when the draft names no `folder`; `"folder": ""` widens it to the whole repository. It never replaces a rule shipped by Rerail. Your copy of one takes no `folder`. It is refused where the repository does not add the original, and where your copy of it is already added; `rules show` names that copy, the one to change. Your copy made for another repository is refused where its original runs and the copy is not added, since both would steer. `rerail rules approve` with no names lists everything awaiting approval and its source.

</details>

The repository pass shows file lines to the agent, so it skips hidden files, secret-like names, nested checkouts and files Git ignores.

<details>
<summary>Exactly what the pass skips</summary>

- hidden files and folders (`.env`, `.ssh/`, `.github/`, any name starting with `.`);
- secret-like names: `*.pem`, `*.key`, `id_rsa*`, `credentials`, `credentials.*`, `credentials-*` or `credentials_*` data files like `credentials-prod.json` (not source like `credentials-helper.ts`), `secrets.*`, `*.tfstate`, `service-account*.json` and the rest of `SENSITIVE_FILE_NAMES` in `src/engine/path-boundary.ts`;
- a nested checkout of its own;
- files Git ignores.

It never follows symlinks; this is enforced in code, not instructions. Tracked files are never ignored. Turning Follow .gitignore off (dashboard Settings or `/api`; agents can only turn it on) lets the pass read ignored files; hidden and secret-named files have no such grant. Unreadable ignore rules make the pass refuse. The report counts skipped files and folders without names (`repository.leftOut`), so a rule about them shows why it matched nothing.

</details>

#### Starting and stopping

Rerail is off in every repository until started. Rules belong to the repository; on/off belongs to each checkout. State lives in `~/.rerail/rerail.sqlite`.

- `rerail start --repository` and `rerail stop --repository` (or the dashboard) act on every checkout.
- Stopping applies immediately in OMP and Pi, and before the next one-shot Claude Code, Codex or Cursor hook. It survives restarts and keeps history, rules and the shared daemon.

<details>
<summary>What stopping does, inheritance and the footer</summary>

A stopped checkout records nothing, runs no rules, delivers no guidance, and its hooks never autostart or contact the daemon.

A checkout without its own value inherits from the nearest containing checkout of the same repository with one, else the repository. Starting or stopping a checkout reaches the checkouts inside it, which follow it again.

In a repository not set up, `/rerail-start` (or `rerail start`) runs set-up first. It recommends packs, adds those you confirm (a language pack at its matched folder, a repository pack for the whole repository) and starts. A started repository that isn't set up records sessions but runs no rules.

The Claude Code, OMP and Pi footer is not shown in a stopped checkout, or one Rerail was never started in. With a daemon running, it shows `R E R A I L ○ not set up · rerail start` in a started repository that isn't set up. `/rerail` and `rerail status` report the checkout's state.

</details>

#### Scan and settings from the terminal

`rerail scan` and `rerail settings` take flags. Outside the coding agent, name the session; Rerail never guesses the latest. Without `--session`, `rerail scan` is a different command, a checkout scan in its own process with no daemon or session: [Headless scan](headless-scan.md).

```sh
rerail settings --session <session-id>
rerail settings --session <session-id> --scope touched-files --mode log
RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1 rerail daemon start   # experimental: the daemon honours the strategy
rerail settings --session <session-id> --steer-strategy rewind
rerail scan --session <session-id> --scope repository --wait
rerail scan --session <session-id> --scope git-diff --wait --json
```

| Exit code | Meaning |
| --- | --- |
| `0` | Submission/completion succeeded. |
| `1` | A request or execution failed or was interrupted. |
| `2` | The wait expired while the scan remained queued/running. |

<details>
<summary>Waiting, output and where to look</summary>

Without `--wait`, the request prints immediately. `--wait` polls up to ten seconds and prints the latest state; `--json` prints the structured record.

Findings alone don't turn successful execution into a failure. Inspect outcomes in Diagnostics by the reported request/run IDs. The Rerail home's daemon serves every recorded session, so the commands work from any directory.

</details>

#### Settings, Diagnostics and Sessions pages

For hosts without native selector components, the standalone UI has **Settings** (ongoing scope and enforcement mode), **Diagnostics** (log-only scans and results) and **Sessions** (transcript events).

The default is automatic `ai-output` audits in `steer` mode with the `message` strategy: an edit hears only about violations in lines it added or changed. `--scope touched-files` reports whole edited files.

<details>
<summary>Page operations, ownership and scan scope</summary>

| Operation                   | HTTP surface                                       |
| --------------------------- | -------------------------------------------------- |
| Effective settings          | `GET /api/sessions/{sessionId}/audit-settings`     |
| Save defaults               | `PATCH /api/sessions/{sessionId}/audit-settings`   |
| Request an audit            | `POST /api/sessions/{sessionId}/audit-requests`    |
| Inspect requests            | `GET /api/sessions/{sessionId}/audit-requests`     |
| Repository audit index      | `GET /api/repository-audits`                       |
| Filtered audit entries      | `GET /api/repository-audits/{auditId}/entries`     |

**Ownership and activation.** Defaults belong to the canonical workspace checkout and are shared by its sessions. They are not shared by other worktrees, conversations, workspaces or a machine-wide preference. Checkout settings are never silently promoted. Rules belong to the repository, so another transcript database doesn't share them ([Rule scoping](rule-scoping.md)). Effective settings name their workspace and storage location.

The default ignores what the rest of an edited file already broke. Saved settings keep their scope. Changes apply to later automatic evaluations, running sessions included, never to a running evaluation or recorded results.

Each repository in retained scan history gets one automatic full-repository, `log`-only baseline, never repeated. If Rerail becomes active in such a repository without one, it runs it before ordinary evaluation. A later user-requested scan becomes the latest Diagnostics result without changing saved defaults. History filters never change evaluation settings.

**Scope and outcomes.** Explicit repository scans cover the whole eligible workspace. Git-diff scans take tracked, staged and untracked changed files at request time; an empty diff fails rather than scanning everything. Automatic AI-output evaluation keeps using captured preimages and changed ranges.

</details>

#### Budgets and coverage gaps

Full-repository work is capped by a five-minute execution budget, a 100,000-file or 512 MiB aggregate source ceiling, and 32-file/1 MiB native batches. Hitting a cap fails the scan with one summary failure, always listed.

- A **coverage gap** is a rule that still runs out of work on a file: not an execution failure.
- A **blind file** is one every rule failed on.

<details>
<summary>Per-file failures and large files</summary>

A single file may use the policy engine's 32 MB source allowance. Unreadable files never stop a scan. At most 20 per-file failures are listed; the audit record and session summary keep the total (`failureCount`, with `unlistedFailureCount` unlisted), and the footer says `audit failed (N failures)`.

Large files have no fixed cut-off:

- A file's shared work (parsing, tape, anchor index) and each clause's allowance scale with its size.
- A clause may visit `maxVisitedNodes` nodes or 16 per file node (`maxVisitsPerNode`), whichever is more.
- Each clause has its own allowance, so a costly rule fails alone.
- A file's clauses may together spend eight allowances per evaluation; a clause finding that spent fails without running.

A rule exhausting its own allowance in a full evaluation of the file is stored and reused by later scans. Failures that depend on what else ran (shared work, the ceiling, a partial evaluation) are re-evaluated next scan.

</details>

<details>
<summary>How gaps and blind files are reported</summary>

A coverage gap's failure suggests simplifying the rule or excluding generated or vendored files. One failure per such rule is kept past the list cap, and the footer counts them. A rule stopped by the file's work (code `file-budget-exceeded`) is also a gap, advised to exclude or split the file, but isn't counted.

A blind file is listed once as a failure marked `blind`, never cut, and reported in `blindFiles` and `blindFileCount`. Blind by work (`budget`) is a gap. Blind by syntax (`syntax`) is a partly read file: a repository audit lists it and completes; a touched-files scan fails.

A scan whose only failures are gaps completes, `rerail scan` exits `0`, and the footer counts unchecked files and the rules that ran out or were stopped, instead of "audit failed". Two cases are not gaps and fail the audit: more than `maxWitnesses` on a file (`witness-limit`), where the rule did read it, and a file past a rule's input limits (`input-limit`), which it never read.

</details>

#### Diagnostics

Diagnostics keeps one-shot requests and results apart from ongoing settings and transcripts; a completed scan with no violations is a successful empty result.

<details>
<summary>Results, filters and host delivery</summary>

- Unreadable inputs and evaluator errors are execution failures, not violations.
- Queued or running requests aren't results; interrupted execution isn't success.
- Requests link to the audit IDs they produced; history keeps session, workspace, source revision and evaluated rule revisions.
- Historical records and work cancelled before evaluation have no compiled-rule provenance, so aren't attributed to today's rules. A recorded catalog with no individual revisions had no enabled code rules.

It indexes every repository in retained session history and shows the latest repository audit's violations and failures as separate accessible records. Filters run in the daemon with bounded pagination; the UI never loads or filters a full result set in memory. Filters are entry type (`kind=violation|failure`), rule mode (`mode=log|steer|block`), exact code, path substring and free text. Rule mode differs from an audit's run mode, filtered by `mode=steer|log` on `/api/audits`.

**Host capabilities.** Only the runtime owning a session executes its requests. OMP/Pi deliver in the background; Claude Code, Codex and Cursor get buffered guidance at their next supported hook event, so a finished scan doesn't prove delivery. Runtime failures stay diagnostics, never guidance. A closed or unavailable session can't be swapped for another.

</details>

## Operate

OMP and Pi manage conversation rules in-session with the `conversation_rules` tool and `/cosine-rules`, both in [internal testing](../README.md#internal-testing).

The tool supports `add`, `remove`, `list` and `test`. Rule changes and delivered-match state are session-scoped and replayable: OMP and Pi append native session entries; event-loop adapters append to the JSONL session ledger.

`/cosine-rules` supports:

```text
/cosine-rules list
/cosine-rules model
/cosine-rules reload
/cosine-rules test <rule-id> <text>
```

Use `test` to tune references and thresholds on positive and negative examples before enabling delivery.

## Semantic workspace

A lower-level programmatic interface to mark, select, embed and compare session content; it and its OMP and Pi tools are in [internal testing](../README.md#internal-testing).

- **`mark`** attaches application-defined metadata to stable content references.
- **`select`** resolves mark expressions plus source, owner, session and time filters.
- **`project`** renders and embeds items, groups, windows or a whole selection, with explicit budgets and lineage.
- **`relate`** compares projections by all-pairs, per-vector top-k or corresponding-group pairing, with optional reductions.

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

OMP and Pi expose these as native tools: `rerail_mark`, `rerail_select`, `rerail_project`, `rerail_relate`.

<details>
<summary>Mark semantics and adapter limits</summary>

These are building blocks, not a finished multiplayer coordination interface; marks stay local unless the caller supplies shared storage and synchronization.

- `ContentRef.span` addresses a UTF-16 text range within an item.
- Marks are exact-target: a mark on an item doesn't apply to its spans.
- Expired marks stay in the append-only ledger but drop out of selections.
- `visibility` is selection metadata, not access control.

Claude Code, Codex and Cursor lifecycle adapters aren't agent-visible tool surfaces; harness registrations call the event-loop interface directly.

</details>
