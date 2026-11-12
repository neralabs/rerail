# Internal testing

> [!WARNING]
> **Not ready for public use.** These features are in the code and you can run them, but we are still testing them
> internally. They are unsupported and may change or disappear in any release without notice. Nothing in Rerail's
> documentation promises they work. Do not build on them or rely on them.

Each is marked `Internal testing:` where defined. Its descriptions (CLI usage, MCP tools, OMP and Pi tools and commands)
start with `[Internal testing]`, via `internalTesting()` in `src/engine/internal-testing.ts`.

| Feature | Where it surfaces |
| --- | --- |
| Full-repository scanning | `rerail scan`; `audit_requests_create`, `audit_requests_list`, `repository_audits_list`, `repository_audit_entries_list` |
| Conversation rules, cosine (similarity) and command (tool-call) | `kind: "conversation"` rules, badged `Internal testing` in the dashboard; `/cosine-rules` and `conversation_rules` in OMP and Pi; `/rerail-create`'s command rules ([Tool-call rules](tool-call-rules.md)); conversation rules a rule pack ships ([pack format](pack-format.md#packjson)) |
| Semantic tools | `rerail_mark`, `rerail_select`, `rerail_project`, `rerail_relate` in OMP and Pi |
| Block mode | `mode: "block"`, stored and delivered exactly like `steer`; CLI and API writes that set it say it steers in this release, the dashboard shows `Block (steers for now)` |
| Activation profiles | `rerail activate`, `rerail activation request`, `rerail profile`, `--profile` |
| Event stream | `rerail events` |
| [Headless scan](headless-scan.md) | `rerail scan` without `--session`: a scan of a checkout with no daemon or session; its `rerail.scan/1` report |
| [Remote sink](remote-sink.md) | `rerail remote`; the daemon's sender; remote lines in `rerail status`; `remote.json`, `remote-tokens/`, `remote-outbox.sqlite`, `machine-id` in the Rerail home |
| [Steer strategies](#steer-strategies) `rewind` and `subagent`, experimental: off unless `RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1` | `steerStrategy` in audit settings (`rerail settings --steer-strategy`, `audit_settings_update`); Pi's `rerail_fix` tool; `benchmark/steer-strategy.ts` |
| Automatic first repository audit, experimental: off unless `RERAIL_EXPERIMENTAL_FIRST_REPOSITORY_AUDIT=1` | One full-repository audit in `log` mode at the first session in a checkout |
| [Missed-steer digest](#missed-steer-digest), experimental: off unless `RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST=1` | Late code-rule steers held and sent as one steer at the agent's yield; Cursor's `stop` follow-up |
| Merge Assist, in-flight conflict detection, off for every repository until a person turns it on ([build](merge-assist-v1-build.md), [reference](reference.md#merge-assist)) | `rerail merge-assist`; the `merge_assist_get`, `merge_assist_settings_get`, `merge_assist_scan`, `merge_assist_settings_set` and `merge_assist_conflict_dismiss` operations; the dashboard's Merge Assist page; its in-session notes ([In-session surfaces](in-session-surfaces.md#merge-assist-notes)) |

- `rerail settings` and the `audit_settings_*` operations are not tagged yet; their scope is undecided.
- The daemon's HTTP API, OpenAPI document and MCP endpoint are in the [reference](reference.md#run); the dashboard uses
  them internally.
- Found something in this list? Feedback is welcome in an issue labelled as internal testing.
- [Governance](GOVERNANCE.md#internal-testing-features) says who decides when a feature graduates or is removed.

## Experimental steer features

Each is off unless its variable is `1` for the process running Rerail's runtime.

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
