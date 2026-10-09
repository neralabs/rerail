# Rerail in-session surfaces

Rerail installs an OMP widget and a Claude Code status line (the HUD) and gives Codex explicit session detail. Product outcomes and acceptance criteria are in the [as-built PRD](in-session-status-line-prd.md).

## Installation and native prerequisites

```sh
npx @neralabs/rerail setup
```

Standard setup registers every surface below. For a source checkout, `node dist/cli.js install --root "$PWD"` registers that build through the same registration code. Restart coding agent sessions afterward. No footer flag, extra OMP extension, prototype launcher or presentation settings file is required.

- Every coding agent gets one command set: `/rerail` (what Rerail is doing here, and every command), `/rerail-ui` (open the dashboard), `/rerail-create` (create a rule in your own words), `/rerail-stop`, `/rerail-start`, `/rerail-update` and `/rerail-rage-quit` (remove Rerail from this computer; rules and history kept in a backup). Settings, audits, status, update checks, rollback and reset are CLI commands only (`rerail settings`, `rerail scan`, `rerail status`, `rerail update check`, `rerail update rollback`, `rerail reset`).
- The default OMP and Pi extensions register the commands natively and own one below-editor footer widget and its lifecycle timers; they do not replace the native footer. `/rerail-create` hands its instructions to the agent; `/rerail-rage-quit` asks with a native selector.
- Claude setup registers hooks, composes `dist/claude-status.js` with existing status output, and writes the commands to `~/.claude/commands/` with exact `allowed-tools` grants. `/rerail-start` also grants set-up's answer commands (`rerail setup …`, through the launcher), so in a repository that is not set up the coding agent relays each question and runs the command printed for the answer without Claude Code asking again. The rage-quit removal (`rerail install --remove`) is not pre-approved, so Claude Code asks before it runs. Native idle refresh defaults to five seconds only when the user has no cadence; unregister removes Rerail's added cadence.
- Cursor setup writes the same commands to `~/.cursor/commands/`.
- Codex setup writes one skill per command under `~/.codex/skills/<name>/SKILL.md`, invoked as `$rerail`, `$rerail-ui`, and so on. Registration removes the `/rerail-session` command file an earlier version wrote when it is Rerail's own. Native hook trust and local-execution approval still apply. Rerail does not install the coding agents themselves.
- SQLite's native binding must match the selected Node ABI. Rerail checks and rebuilds it before starting the daemon when necessary; this is a setup prerequisite, not another presentation store.
- Pi's direct packages are aligned to 0.85.1, and its existing integration harness uses `ModelRuntime` with in-memory credentials rather than removed `AuthStorage`.
- Repository discovery remains background work so OMP readiness and path-aware tools never wait for it. Opaque/pathless tools also start immediately; a command tool (`bash`, `shell`, `exec`, …) is noted at `tool_call`, and its `tool_result`, even a failed one, waits for a discovery pass that reconciles what the shell wrote. Passes are anchored to a filesystem-clock reading taken before the walk, so a source stamped at or after session start is reported as an uncaptured change rather than silently baselined; a deletion is reported only for a source the baseline had already observed.
- That reconciliation wait has no local timer in OMP/Pi's direct awaited `tool_result` path. One-shot hook transport retains its 30-second request timeout; turn and final checkpoints likewise wait for the walk, and a walk that failed or was cancelled reports a `source-incomplete` failure instead of partial coverage.

Claude's native settings files contain integration wiring, not presentation preferences. Ownership and the complete predecessor specification live in `~/.rerail/harness.sqlite`, separate from project migrations. Generated inherited wiring resolves the invocation workspace's local/shared command dynamically, then global fallback; genuine user-local predecessors retain their precedence. Shared edits need no reinstall. Registration and removal preserve unrelated settings and later user replacements without nesting wrappers or duplicating global hooks into project files.

## Host capabilities

| Coding agent | Footer / explicit surface | Identity and limits |
| --- | --- | --- |
| Claude Code | Status-line command receives JSON on stdin and emits text, ANSI and OSC 8; `/rerail` for detail | Native `session_id` and `cwd`. Installed terminal proof on 2.1.269 included three links and brand color; activation depends on terminal support. |
| OMP / Pi | `ctx.ui.setWidget(key, content, { placement: "belowEditor" })`; `/rerail` for detail | `ctx.sessionManager.getSessionId()` and `ctx.cwd`. Actual linked OMP output captured on 18.1.15. `setStatus` strips ANSI/OSC 8; the widget preserves them. Inspected `setFooter`/`setHeader` were no-ops. |
| Codex | `$rerail` and the other `$rerail-*` skills; no footer | Exact native thread identity, never the latest session in a directory. Inspected status-line items were a closed built-in set; this limitation is version-sensitive. |
| Cursor | `/rerail`; no footer | Cursor cannot name its session, so `/rerail` shows the repository-level form (`rerail session cursor`). |

A skill is an on-demand procedure, not a heartbeat or security boundary. Do not simulate an ambient Codex footer through injected messages.

### Session-start note

In a repository where Rerail is started, each session begins with a short note to the agent: Rerail is on, what a steer looks like, the session's dashboard link, and that `/rerail` (Codex: `$rerail`) lists the commands. Claude Code and Codex receive it as `SessionStart` `additionalContext`, Cursor as `sessionStart` `additional_context`, and OMP and Pi as a hidden `before_agent_start` message (`customType: "rerail.session-note"`). Rerail never writes instruction files.

### `/rerail`

`/rerail` is rendered by `rerail session [<host> [<session-id>]]`:

```text
● Rerail is on in this repo · acme-web
  26 rules · steered 1× in this session
  See what it logged → http://127.0.0.1:4737/?sessionId=…&host=claude-code#sessions

Commands
/rerail-ui           open the dashboard
…
Or just ask me, in plain words.
```

### Steer

Each steered rule is one block:

```text
▲ Rerail · <rule id>  Steered.
<guidance; for file rules: message (path:line:col), then any Suggested fix line>
Why? → <dashboard>/#rule/<id>
```

Claude Code and Codex also show the person one line per rule (`systemMessage`): `▲ Rerail · <id>  Steered. Why? → <link>`. Codex delivers at `PostToolUse`, with `Stop` as the catch-up. Cursor shows the person nothing, so the guidance asks the agent to relay the line. OMP and Pi show the block in the session.

The runtime records the link relative (`#rule/<id>`). Each coding agent's adapter makes it absolute as it hands the steer over, using the same dashboard the footer links to: the Rerail daemon's recorded URL when it is loopback, scoped to the session's repository and checkout. The `rerail events` stream does the same for its harness, using the session's cwd (else the stream's working directory). A daemon bound to `0.0.0.0`, or no daemon at all, leaves the link relative rather than pointing a browser at an address it cannot open. The session-start note and `rerail rules add` resolve their links the same way.

Primary references: [Claude status lines](https://code.claude.com/docs/en/statusline), [skills](https://code.claude.com/docs/en/skills), [settings/worktree precedence](https://code.claude.com/docs/en/settings#where-claude-code-keeps-the-local-file-in-a-git-repository), [Codex configuration](https://developers.openai.com/codex/config-reference), and [inspected Codex status-line source](https://github.com/openai/codex/blob/a1294e57f12e9474088a2f6e6a9142ed3e1ddccd/codex-rs/tui/src/bottom_pane/status_line_setup.rs). OMP source inspection covered extension types, the extension UI controller and status-line segments of the OMP version pinned in `package-lock.json`.

## What the footer means

```text
Rerail ● 26 rules  ·  steered 3×
```

`  ·  steered N×` appears only above zero. Other states: `Rerail ○ not on here` where Rerail is not started or was stopped, `Rerail ✕ not running` when no daemon answers, and `Rerail ● session unavailable` before the session is recorded. `engine unavailable` and `audit failed` are appended when relevant. Claude Code (status line), OMP and Pi (below-editor widget) show it; Codex and Cursor have no footer, and `/rerail` (`$rerail`) is the way in.

- **Rules** is configured session coverage: the effective conversation-rule registry and enabled compiled code-rule catalog, including host options/files, database overrides and session mutations. It is not all enabled database rows, rules matching the current file, or successful checks.
- **Steered** counts successful native `sendMessage` steers or successfully emitted hook responses with nonblank guidance. Bundled guidance is one steer. Observations, matches, queued/deferred guidance and failed sends do not count. Receipt retries deduplicate; a new actual steer counts again, against its originating session.
- **Unknown is not zero.** Older sessions and histories lost in a reset cannot establish complete totals and show `steering unavailable`. Invalidated coverage shows `rules unavailable` until the host synchronizes. Routine streaming deltas do not recompute coverage.
- A steer proves neither model consumption nor correction/recheck success. A crash or unrecoverable receipt-write failure after delivery can undercount: host delivery and the SQLite write are not atomic.
- The footer may append `checking…`; Engine/session unavailability and audit failures stay explicit. Violations are not an outstanding-work queue or evidence that changes are ready for human review. Explicit detail retains health, scope/status, violation/failure totals and `not evaluated` when no run exists.

`Rerail` and the steering count link to the exact current session; the rule count links to `/#rules` on the same daemon. Only `Rerail` uses the brand colour. The terminal token is generated from the same CSS tokens as the UI (`src/terminal-brand.generated.ts`), so installed packages need no second palette.

Presentation reads are bounded and passive: one HTTP request validates daemon identity, resolves presence and returns exact-session HUD counts with a bounded latest-evaluation summary. They never start/retarget a daemon, trigger evaluation or open a writable database. Full audit bodies and rule lists belong to explicit dashboard reads. The installed passive reader is tree-shaken at build time while retaining schema-derived validation.

## Session links and receiving route

```text
http://127.0.0.1:<daemon-port>/?sessionId=<encoded-id>&host=<encoded-host>#sessions
```

The reader discovers the daemon for the originating database, validates loopback origin and canonical database, then verifies daemon PID/database and the session ID, host and working directory. Redirects are rejected and reads are bounded. Locator state is re-read on refresh so daemon restarts need not leave stale links.

The existing browser route uses the URL for both view and selection, including reloads and Back/Forward. Missing sessions, missing hosts and host mismatches show unavailability, never an arbitrary/latest session. Selecting a recorded session writes both ID and host. Rules, settings and diagnostics navigation remain intact; legacy `#scans` resolves to diagnostics.

Every session records into the Rerail home's one database, so the daemon answers status for sessions of any repository. Its Rules page and listings belong to the repository it was started in; `rerail ui` from another repository reports `daemon serves another repository` rather than linking to the wrong Rules page. Bookmarks are not durable cross-database identities or synchronized settings. A recorded `live` label means no end timestamp; it is not proof that a coding-agent process is still running.

## SQLite-owned presentation preferences

Presentation is independent of enforcement: `silent` hides ambient UI but does not disable policies, suppress guidance or change blocking. Explicit `/rerail`, `$rerail` and CLI/API detail remain available. Do not combine `silent / ambient / steer / block` into one preference ladder.

The existing project schema owns `presentation_preferences`, keyed by `(repository, host)` with presence `compact` or `silent`. `repository` is a canonical absolute path (empty means all repositories); `host` uses Rerail's supported host vocabulary (empty means all hosts). Cursor is valid for explicit resolution without acquiring a status-line footer. These defaults belong only to this database, not other projects or synchronized users.

Precedence is exact repository + host, repository default (`host = ""`), host default (`repository = ""`), database default (both empty), then built-in `compact`. Removing an override reveals the next default.

Canonicalization includes existing ancestors of not-yet-created paths and unifies symlink spellings such as `/tmp` and `/private/tmp`. It does not infer repository renames or cross-machine identity.

The schema-derived daemon API exposes:

| Operation | Purpose |
| --- | --- |
| `GET /api/presentation-preferences` | List stored records |
| `PUT /api/presentation-preferences` | Upsert `{ repository, host, presence }` |
| `DELETE /api/presentation-preferences?repository=…&host=…` | Remove an override and reveal the next default |
| `GET /api/presentation?repository=…&host=…[&sessionId=…&onDemand=true]` | Resolve presence; optionally include exact-session context |

Only the daemon owns writable preference access; preferences survive reset. There is no separate presentation file or dedicated end-user preference screen. OMP refreshes approximately every five seconds; Claude respects its native cadence. An unreachable daemon can prevent preference resolution and show an unavailable indicator even if `silent` was previously stored.

## Boundaries

- Commands run by a command tool wait for a reconciling discovery pass before their results are evaluated; that wait has no separate measured bound against the 30-second one-shot hook deadline. Shell writes made before the baseline reaches their files are reported as uncaptured changes; deletions of never-observed sources remain outside that guarantee.
- Claude status composition selects each workspace's shared command in a Git main/worktree pair, observes shared edits without reinstalling, honours genuine local precedence and global fallback, and preserves predecessor commands larger than the JSON budget.
- Session links prove route behavior in the bundled browser (exact, missing, wrong-host, missing-host and loading states, Back/Forward); terminal hyperlink activation depends on the terminal.
- None of this is a production reliability guarantee or evidence of model compliance. No Pi status-line journey, terminal pointer activation or Codex approval bypass is claimed.
