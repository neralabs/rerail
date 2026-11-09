# Internal testing

> [!WARNING]
> **These features are in the code and you can find and run them, but they are not ready for public use.** We are
> still testing them internally. They are not supported, they may change or be removed in any release without notice,
> and nothing in Rerail's documentation promises they work. Do not build on them or rely on them in your workflow.

Each is tagged with an `Internal testing:` comment where it is defined, and every description a person or a coding agent reads for one (CLI usage, MCP tools, OMP and Pi tools and commands) starts with `[Internal testing]`, through `internalTesting()` in `src/engine/internal-testing.ts`.

| Feature | Where it surfaces |
| --- | --- |
| Full-repository scanning | `rerail scan`; the `audit_requests_create`, `audit_requests_list`, `repository_audits_list` and `repository_audit_entries_list` operations |
| Conversation rules, cosine (similarity) and command (tool-call) alike | `kind: "conversation"` rules, shown in the dashboard with an `Internal testing` badge; `/cosine-rules` and the `conversation_rules` tool in OMP and Pi; `/rerail-create`'s command rules ([Tool-call rules](tool-call-rules.md)) |
| Semantic tools | `rerail_mark`, `rerail_select`, `rerail_project` and `rerail_relate` in OMP and Pi |
| Block mode | `mode: "block"`, stored and delivered exactly like `steer`; the CLI and API writes that set it say it steers in this release, and the dashboard shows it as `Block (steers for now)` |
| Activation profiles | `rerail activate`, `rerail activation request`, `rerail profile` and `--profile` |
| Event stream | `rerail events` |
| Headless scan ([Headless scan](headless-scan.md)) | `rerail scan` without `--session`: a scan of a checkout with no daemon or session, and its `rerail.scan/1` report |
| Remote sink ([Remote sink](remote-sink.md)) | `rerail remote`; the daemon's sender; the remote lines in `rerail status`; the files `remote.json`, `remote-tokens/`, `remote-outbox.sqlite` and `machine-id` in the Rerail home |
| Steer strategies `rewind` and `subagent`, experimental: off unless `RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1` | `steerStrategy` in the audit settings (`rerail settings --steer-strategy`, `audit_settings_update`); Pi's `rerail_fix` tool; `benchmark/steer-strategy.ts` |
| Automatic first repository audit, experimental: off unless `RERAIL_EXPERIMENTAL_FIRST_REPOSITORY_AUDIT=1` | One full-repository audit in `log` mode at the first session in a checkout |
| Missed-steer digest, experimental: off unless `RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST=1` | Late code-rule steers held and sent as one steer at the agent's yield; Cursor's `stop` follow-up |

`rerail settings` and the `audit_settings_*` operations are not tagged yet: their scope is still being decided. The daemon's HTTP API, OpenAPI document and MCP endpoint are described in the [reference](reference.md#run); the dashboard uses them internally. Found something in this list? Feedback is welcome in an issue, labelled as internal testing. Who decides when a feature graduates or is removed is in [Governance](GOVERNANCE.md#internal-testing-features).

## Experimental steer features

Each is off unless its variable is `1` in the environment of the process that runs Rerail's runtime: the daemon for OMP, Claude Code, Codex and Cursor, or the coding agent itself for a Pi session running Rerail in process.

**Missed-steer digest** (`RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST=1`; Cursor's follow-up also needs it in Cursor's hook environment). A steer whose check outlasts its step's wait is held instead of reaching some later, unrelated step. When the agent yields (ends its run), it gets one steer led by `Rerail finished checking your earlier edits after you moved on:`, listing every held finding once, in arrival order; a file edited again meanwhile drops out. Findings that arrive after a yield wait for the next one. In Cursor the digest is a `stop` follow-up, which Cursor submits as the next message, at most three times in a row. The digest has no size limit.

**Steer strategies** (`RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1`). The checkout's `steerStrategy` (`rerail settings --steer-strategy`) decides how a code-rule steer reaches the agent; off, every steer is `message`.

| Strategy | What the agent gets |
| --- | --- |
| `message` (default) | The steer as in [in-session surfaces](in-session-surfaces.md#steer). |
| `rewind` | When the edit's own check steers before its tool result is answered, Rerail restores every file the call changed (deleting files it created), all or none, and the steer says what it put back and to redo the edit following the rule. OMP and Pi put it in that tool result, marked as an error; Claude Code, Codex and Cursor carry it like any steer. A steer found later, or for an edit Rerail cannot restore, goes out as `message`. |
| `subagent` | The steer, plus a note that it comes from Rerail, the agent's task is unchanged, and to hand the fix to a subagent: `task` in OMP, `Task` in Claude Code and Cursor, `spawn_agent` in Codex, and `rerail_fix` in Pi. `rerail_fix` runs a child `pi` with the session's model, Pi's file tools, no shell and no extensions, and stops it after 10 minutes. |

`benchmark/steer-strategy.ts` measures the three on real coding agents.
