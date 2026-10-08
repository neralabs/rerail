# Rerail in-session surfaces

Rerail installs an OMP widget and a Claude Code status line (the HUD) and gives Codex explicit session detail. Product outcomes and acceptance criteria are in the [as-built PRD](in-session-status-line-prd.md).

## Installation and native prerequisites

```sh
npx @neralabs/rerail setup
```

Standard setup registers every surface below. For a source checkout, `node dist/cli.js install --root "$PWD"` registers that build through the same registration code. Restart coding-agent sessions afterward. No HUD flag, extra OMP extension, prototype launcher or presentation settings file is required.

- The default OMP extension owns one `rerail` below-editor widget and its lifecycle timers; it does not replace the footer. `/rerail` gives exact-session detail. Existing `/rerail-*` settings, diagnostics and maintenance commands remain separate.
- Claude setup registers hooks and composes `dist/claude-status.js` with existing status output. `/rerail` reads the current session; explicit launcher actions remain available. Native idle refresh defaults to five seconds only when the user has no cadence; unregister removes Rerail's added cadence.
- Setup packages the Codex `$rerail` skill. Native hook trust and local-execution approval still apply. Rerail does not install the coding-agent applications themselves.
- SQLite's native binding must match the selected Node ABI. Rerail checks and rebuilds it before starting the daemon when necessary; this is a setup prerequisite, not another presentation store.
- Pi's direct packages are aligned to 0.85.1, and its existing integration harness uses `ModelRuntime` with in-memory credentials rather than removed `AuthStorage`. These delivered compatibility changes preserve the Pi integration; they do not establish a new Pi status-line surface or a visually verified Pi journey.
- Repository discovery remains background work so OMP readiness and path-aware tools never wait for it. Opaque/pathless tools also start immediately; a command tool (`bash`, `shell`, `exec`, …) is noted at `tool_call`, and its `tool_result`, even a failed one, waits for a discovery pass that reconciles what the shell wrote. Passes are anchored to a filesystem-clock reading taken before the walk, so a source stamped at or after session start is reported as an uncaptured change rather than silently baselined; a deletion is reported only for a source the baseline had already observed.
- That reconciliation wait has no local timer in OMP/Pi's direct awaited `tool_result` path. One-shot hook transport retains its 30-second request timeout; turn and final checkpoints likewise wait for the walk, and a walk that failed or was cancelled reports a `source-incomplete` failure instead of partial coverage.

Claude's native settings files contain integration wiring, not presentation preferences. Ownership and the complete predecessor specification live in `~/.rerail/harness.sqlite`, separate from project migrations. Generated inherited wiring resolves the invocation workspace's local/shared command dynamically, then global fallback; genuine user-local predecessors retain their precedence. Shared edits need no reinstall. Registration and removal preserve unrelated settings and later user replacements without nesting wrappers or duplicating global hooks into project files.

## Host capabilities

| Host | Status line / explicit surface | Identity and limits |
| --- | --- | --- |
| Claude Code | Status-line command receives JSON on stdin and emits text, ANSI and OSC 8; `/rerail` for detail | Native `session_id` and `cwd`. Installed terminal proof on 2.1.269 included three links and brand color; activation depends on terminal support. |
| OMP | `ctx.ui.setWidget(key, content, { placement: "belowEditor" })`; `/rerail` for detail | `ctx.sessionManager.getSessionId()` and `ctx.cwd`. Actual linked output captured on 18.1.15. `setStatus` strips ANSI/OSC 8; the widget preserves them. Inspected `setFooter`/`setHeader` were no-ops. |
| Codex | Explicit `$rerail` skill; no persistent Rerail footer | Exact native thread identity, never the latest session in a directory. Skill proof on 0.154.0. Inspected status-line items were a closed built-in set; this limitation is version-sensitive. |
| Pi / Cursor | Existing integrations retained; no new status-line surface claimed | Shared CLI/API capabilities do not establish a newly verified native UI journey. |

A skill is an on-demand procedure, not a heartbeat or security boundary. Do not simulate a Codex status line through injected messages or duplicate native commands with drifting skills.

Primary references: [Claude status lines](https://code.claude.com/docs/en/statusline), [skills](https://code.claude.com/docs/en/skills), [settings/worktree precedence](https://code.claude.com/docs/en/settings#where-claude-code-keeps-the-local-file-in-a-git-repository), [Codex configuration](https://developers.openai.com/codex/config-reference), and [inspected Codex status-line source](https://github.com/openai/codex/blob/a1294e57f12e9474088a2f6e6a9142ed3e1ddccd/codex-rs/tui/src/bottom_pane/status_line_setup.rs). OMP source inspection covered extension types, the extension UI controller and status-line segments of the OMP version pinned in `package-lock.json`.

## What the HUD means

```text
Rerail | 12 rules | steered 3×
```

- **Rules** is configured session coverage: the effective conversation-rule registry and enabled compiled code-rule catalog, including host options/files, database overrides and session mutations. It is not all enabled database rows, rules matching the current file, or successful checks.
- **Steered** counts successful native `sendMessage` steers or successfully emitted hook responses with nonblank guidance. Bundled guidance is one steer. Observations, matches, queued/deferred guidance and failed sends do not count. Receipt retries deduplicate; a new actual steer counts again, against its originating session.
- **Unknown is not zero.** Older sessions and histories lost in a reset cannot establish complete totals and show `steering unavailable`. Invalidated coverage shows `rules unavailable` until the host synchronizes. Routine streaming deltas do not recompute coverage.
- A steer proves neither model consumption nor correction/recheck success. A crash or unrecoverable receipt-write failure after delivery can undercount: host delivery and the SQLite write are not atomic.
- The HUD may append `checking…`; engine/session unavailability and evaluation failures stay explicit. Violations are not an outstanding-work queue or evidence that changes are ready for human review. Explicit detail retains health, scope/status, violation/failure totals and `not evaluated` when no run exists.

`Rerail` and the steer count link to the exact current session; the rule count links to `/#rules` on the same daemon. Only `Rerail` uses the brand colour. The terminal token is generated from the same CSS tokens as the UI (`src/terminal-brand.generated.ts`), so installed packages need no second palette.

Presentation reads are bounded and passive: one HTTP request validates daemon identity, resolves presence and returns exact-session HUD counts with a bounded latest-evaluation summary. They never start/retarget a daemon, trigger evaluation or open a writable database. Full audit bodies and rule lists belong to explicit dashboard reads. The installed passive reader is tree-shaken at build time while retaining schema-derived validation.

## Session links and receiving route

```text
http://127.0.0.1:<daemon-port>/?sessionId=<encoded-id>&host=<encoded-host>#sessions
```

The reader discovers the daemon for the originating database, validates loopback origin and canonical database, then verifies daemon PID/database and the session ID, host and working directory. Redirects are rejected and reads are bounded. Locator state is re-read on refresh so daemon restarts need not leave stale links.

The existing browser route uses the URL for both view and selection, including reloads and Back/Forward. Missing sessions, missing hosts and host mismatches show unavailability, never an arbitrary/latest session. Selecting a recorded session writes both ID and host. Rules, settings and diagnostics navigation remain intact; legacy `#scans` resolves to diagnostics.

Every session records into the Rerail home's one database, so the daemon answers status for sessions of any repository. Its Rules page and listings belong to the repository it was started in; `rerail ui` from another repository reports `daemon serves another repository` rather than linking to the wrong Rules page. Bookmarks are not durable cross-database identities or synchronized settings. A recorded `live` label means no end timestamp; it is not proof that a coding-agent process is still running.

## SQLite-owned presentation preferences

Presentation is independent of enforcement: `silent` hides the status line but does not disable rules, suppress guidance or change blocking. Explicit `/rerail`, `$rerail` and CLI/API detail remain available. Do not combine `silent / ambient / steer / block` into one preference ladder.

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
