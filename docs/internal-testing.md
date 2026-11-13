# Internal testing

> [!WARNING]
> **Not ready for public use.** These features are in the code and you can run them, but we are still testing them
> internally. They are unsupported and may change or disappear in any release without notice. Nothing in Rerail's
> documentation promises they work. Do not build on them or rely on them.

Each is marked `Internal testing:` where defined. Its descriptions (CLI usage, MCP tools, OMP and Pi tools and commands)
start with `[Internal testing]`, via `internalTesting()` in `src/engine/internal-testing.ts`.

Every one is experimental and off by default: its variable turns it on ([below](#turning-experimental-features-on)).

| Feature, and its variable | Where it surfaces | While it is off |
| --- | --- | --- |
| Full-repository scanning, `RERAIL_EXPERIMENTAL_REPOSITORY_SCAN` (daemon) | `rerail scan --session`; `audit_requests_create`, `audit_requests_list`, `repository_audits_list`, `repository_audit_entries_list` | The operations answer `404 experimental-off` and are no MCP tools, so `rerail scan --session` fails with that message |
| [Headless scan](headless-scan.md), `RERAIL_EXPERIMENTAL_HEADLESS_SCAN` (the CLI) | `rerail scan` without `--session`: a scan of a checkout with no daemon or session; its `rerail.scan/1` report | `rerail scan` without `--session` refuses |
| Conversation rules, cosine (similarity) and command (tool-call), `RERAIL_EXPERIMENTAL_CONVERSATION_RULES` (daemon; OMP or Pi for their tools; the CLI for drafts) | `kind: "conversation"` rules, badged `Internal testing` in the dashboard; `/cosine-rules` and `conversation_rules` in OMP and Pi; `/rerail-create`'s command rules ([Tool-call rules](tool-call-rules.md)); conversation rules a rule pack ships ([pack format](pack-format.md#packjson)) | None is checked, wherever it is stored; the dashboard lists them as `Not running`. `library_rules_upsert` answers `400 experimental-off` for one, `rerail rules test` and `rules add` refuse its draft, and OMP and Pi register neither the tool nor the command |
| Semantic tools, `RERAIL_EXPERIMENTAL_SEMANTIC_TOOLS` (OMP or Pi; the daemon for OMP's calls) | `rerail_mark`, `rerail_select`, `rerail_project`, `rerail_relate` in OMP and Pi | OMP and Pi do not register them |
| Block mode, `RERAIL_EXPERIMENTAL_BLOCK_MODE` (daemon; the CLI for drafts) | `mode: "block"`, stored and delivered exactly like `steer`; CLI and API writes that set it say it steers in this release, the dashboard shows `Block (steers for now)` | A write that sets it answers `400 experimental-off` (`repository_settings_set`, `repository_attachments_set`, `library_rules_upsert`) or is refused (`rerail rules test`, `rules add`), and the dashboard shows no Block button; the refusal says to save the rule on `steer`, which does the same in this release. A rule already on Block keeps it, and steers |
| Activation profiles, `RERAIL_EXPERIMENTAL_ACTIVATION_PROFILES` (the CLI) | `rerail activate`, `rerail activation request`, `rerail profile`, `--profile` | Each refuses, and no stored profile restricts `rerail update` |
| Event stream, `RERAIL_EXPERIMENTAL_EVENT_STREAM` (the CLI) | `rerail events` | `rerail events` refuses. The built `dist/harness/event-stream.js`, which `rerail update`'s health check runs, and the package's `serveRerailEventStream` stay |
| [Remote sink](remote-sink.md), `RERAIL_EXPERIMENTAL_REMOTE_SINK` (daemon for sending; the CLI for its commands) | `rerail remote`; the daemon's sender; remote lines in `rerail status`; `remote.json`, `remote-tokens/`, `remote-outbox.sqlite`, `machine-id` in the Rerail home | The daemon sends nothing, a headless scan queues no report, and `rerail remote` refuses but for `list`, `status` and `remove`, so a remote added earlier can always be seen and taken away |
| [Steer strategies](#steer-strategies) `rewind` and `subagent`, `RERAIL_EXPERIMENTAL_STEER_STRATEGIES` (daemon) | `steerStrategy` in audit settings (`rerail settings --steer-strategy`, `audit_settings_update`); Pi's `rerail_fix` tool; `benchmark/steer-strategy.ts` | Every steer goes out as `message`, whatever the checkout saved |
| Automatic first repository audit, `RERAIL_EXPERIMENTAL_FIRST_REPOSITORY_AUDIT` (daemon) | One full-repository audit in `log` mode at the first session in a checkout | No such audit |
| [Missed-steer digest](#missed-steer-digest), `RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST` (daemon; Cursor's hook) | Late code-rule steers held and sent as one steer at the agent's yield; Cursor's `stop` follow-up | A late steer reaches the agent at its next event or poll |
| [Merge Assist](#merge-assist), in-flight conflict detection, `RERAIL_EXPERIMENTAL_MERGE_ASSIST` (daemon; the CLI for `rerail merge-assist`), then off for every repository until a person turns it on ([build](merge-assist-v1-build.md), [reference](reference.md#merge-assist)) | `rerail merge-assist`; the `merge_assist_get`, `merge_assist_settings_get`, `merge_assist_scan`, `merge_assist_settings_set` and `merge_assist_conflict_dismiss` operations; the dashboard's Merge Assist page; its in-session notes ([In-session surfaces](in-session-surfaces.md#merge-assist-notes)) | Nothing of it runs; its operations answer `404 experimental-off` and are no MCP tools, and `rerail merge-assist` refuses |

An operation of a feature that is off is left out of the MCP tool list, so a coding agent is never offered a tool that
would only refuse; the HTTP API and the OpenAPI document keep it. The daemon's `/healthz` lists the features it runs
(`experiments`), and the dashboard shows only theirs.

- `rerail settings` and the `audit_settings_*` operations are not tagged yet; their scope is undecided.
- The daemon's HTTP API, OpenAPI document and MCP endpoint are in the [reference](reference.md#run); the dashboard uses
  them internally.
- Found something in this list? Feedback is welcome in an issue labelled as internal testing.
- [Governance](GOVERNANCE.md#internal-testing-features) says who decides when a feature graduates or is removed.

## Turning experimental features on

Every experimental feature in the table is off by default. Each has its own variable, `RERAIL_EXPERIMENTAL_<FEATURE>`,
and `RERAIL_EXPERIMENTAL=1` turns on every one of them. A feature's own variable wins: `1` turns it on, `0` keeps it
off even with `RERAIL_EXPERIMENTAL=1`; unset (or any other value), the feature follows `RERAIL_EXPERIMENTAL`. For
example, `RERAIL_EXPERIMENTAL=1 RERAIL_EXPERIMENTAL_STEER_STRATEGIES=0` runs every experimental feature but steer
strategies (`src/engine/experimental-features.ts`).

Set them for the process that runs the feature, which the table names in brackets. For the daemon, a daemon already
running keeps the environment it started with: stop it and start it again with the variable, as in
`rerail daemon stop && RERAIL_EXPERIMENTAL=1 rerail daemon start`. Their behaviour may change or go away.

## Experimental steer features

Each is off unless it is turned on ([above](#turning-experimental-features-on)) for the process running Rerail's
runtime.

That is the daemon for OMP, Claude Code, Codex and Cursor. For a Pi session, which runs Rerail in process, it is the
coding agent itself.

### Missed-steer digest

Set `RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST=1`; for Cursor's follow-up, also in Cursor's hook environment.

- A steer whose check outlasts its step's wait is held, not sent to a later, unrelated step.
- When the agent yields (ends its run), it gets one steer led by
  `Rerail finished checking your earlier edits after you moved on:`, listing each held finding once, in arrival order.

<details>
<summary>Edge cases</summary>

- Files edited again meanwhile drop out. Findings that arrive after a yield wait for the next one.
- In Cursor, the digest is a `stop` follow-up, which Cursor submits as the next message, at most three times in a row.
- The digest has no size limit.

</details>

### Steer strategies

Set `RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1`. The checkout's `steerStrategy` (`rerail settings --steer-strategy`) then
picks how a code-rule steer reaches the agent; off, every steer is `message`.

| Strategy | What the agent gets |
| --- | --- |
| `message` (default) | The steer as in [in-session surfaces](in-session-surfaces.md#steer). |
| `rewind` | Its edit undone, and a steer to redo it following the rule. |
| `subagent` | The steer, plus a note to hand the fix to a subagent. |

<details>
<summary>How rewind and subagent work</summary>

**`rewind`** applies when the edit's own check steers before its tool result is answered.

- Rerail restores every file the call changed, all or none, deleting files it created. The steer says what it put back.
- OMP and Pi put the steer in that tool result, marked as an error. Claude Code, Codex and Cursor carry it like any
  steer.
- A steer found later, or one for an edit Rerail cannot restore, goes out as `message`.

**`subagent`** notes that the steer comes from Rerail and the agent's task is unchanged.

- The subagent tool is `task` in OMP, `Task` in Claude Code and Cursor, `spawn_agent` in Codex, `rerail_fix` in Pi.
- `rerail_fix` runs a child `pi` with the session's model, Pi's file tools, no shell and no extensions, and stops it
  after 10 minutes.

</details>

`benchmark/steer-strategy.ts` measures the three strategies on real coding agents.

## Merge Assist

Merge Assist is experimental and off by default. Set `RERAIL_EXPERIMENTAL_MERGE_ASSIST=1` (or `RERAIL_EXPERIMENTAL=1`,
[above](#turning-experimental-features-on)) for the Rerail daemon (it
runs the sessions' detection, the API and the dashboard) and for the shell that runs `rerail merge-assist`; then a
person still turns it on per repository (`rerail merge-assist on`, or the dashboard's Merge Assist page).

Without the variable nothing of it runs, even in a repository where it was turned on before: no detection, no `gh`, no
hook command, no notes to the agent and no footer segment. The `merge_assist_*` operations answer
`404 experimental-off`, `rerail merge-assist` exits 1 naming the variable, and the dashboard shows no Merge Assist item
in its sidebar.
