# Rerail in-session surfaces

[NER-1464](https://linear.app/nera/issue/NER-1464/spike-surface-rerail-in-agent-sessions-configurably-and-link-back-to) adds installed OMP and Claude Code HUDs and explicit Codex session detail. This checkout has been installed locally; a public package release is still required for ordinary registry installs to receive it. Product outcomes and acceptance criteria are in the [as-built PRD](in-session-status-line-prd.md).

## Installation and native prerequisites

```sh
npx @neralabs/rerail setup
just rerail
```

`just rerail` wraps standard setup. For an unreleased checkout, use `just rerail-dev` to build, register and restart that checkout through the same registration code. Restart coding-agent sessions afterward. No HUD flag, extra OMP extension, prototype launcher or presentation settings file is required.

- The default OMP extension owns one `rerail` below-editor widget and its lifecycle timers; it does not replace the footer. `/rerail` gives exact-session detail. Existing `/rerail-*` settings, diagnostics and maintenance commands remain separate.
- Claude setup registers hooks and composes `dist/claude-status.js` with existing status output. `/rerail` reads the current session; explicit launcher actions remain available. Native idle refresh defaults to five seconds only when the user has no cadence; unregister removes Rerail's added cadence.
- Setup packages the Codex `$rerail` skill. Native hook trust and local-execution approval still apply. Rerail does not install the coding-agent applications themselves.
- SQLite's native binding must match the selected Node ABI. The development installer checks and rebuilds it before registration when necessary; this is a setup prerequisite, not another presentation store.
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

Primary references: [Claude status lines](https://code.claude.com/docs/en/statusline), [skills](https://code.claude.com/docs/en/skills), [settings/worktree precedence](https://code.claude.com/docs/en/settings#where-claude-code-keeps-the-local-file-in-a-git-repository), [Codex configuration](https://developers.openai.com/codex/config-reference), and [inspected Codex status-line source](https://github.com/openai/codex/blob/a1294e57f12e9474088a2f6e6a9142ed3e1ddccd/codex-rs/tui/src/bottom_pane/status_line_setup.rs). OMP source inspection covered extension types, the extension UI controller and status-line segments; the [original proof lockfile](https://github.com/neralabs/nera-landing/blob/783c13f477fb82032e61c0da5d89df094c44182f/rerail/package-lock.json) identifies its runtime.

## What the HUD means

```text
Rerail | 12 rules | steered 3×
```

- **Rules** is configured session coverage: the effective conversation-rule registry and enabled compiled code-rule catalog, including host options/files, database overrides and session mutations. It is not all enabled database rows, rules matching the current file, or successful checks.
- **Steered** counts successful native `sendMessage` steers or successfully emitted hook responses with nonblank guidance. Bundled guidance is one steer. Observations, matches, queued/deferred guidance and failed sends do not count. Receipt retries deduplicate; a new actual steer counts again, against its originating session.
- **Unknown is not zero.** Older sessions and histories lost in a reset cannot establish complete totals and show `steering unavailable`. Invalidated coverage shows `rules unavailable` until the host synchronizes. Routine streaming deltas do not recompute coverage.
- A steer proves neither model consumption nor correction/recheck success. A crash or unrecoverable receipt-write failure after delivery can undercount: host delivery and the SQLite write are not atomic.
- The HUD may append `checking…`; engine/session unavailability and evaluation failures stay explicit. Violations are not an outstanding-work queue or evidence that changes are ready for human review. Explicit detail retains health, scope/status, violation/failure totals and `not evaluated` when no run exists.

`Rerail` and the steer count link to the exact current session; the rule count links to `/#rules` on the same daemon. Only `Rerail` uses Nera's canonical `--brand`. `build:ts` generates the terminal token from CSS so installed packages need neither a Landing checkout nor a second palette.

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

The schema-derived daemon API and generated Hey API client expose:

| Operation | Purpose |
| --- | --- |
| `GET /api/presentation-preferences` | List stored records |
| `PUT /api/presentation-preferences` | Upsert `{ repository, host, presence }` |
| `DELETE /api/presentation-preferences?repository=…&host=…` | Remove an override and reveal the next default |
| `GET /api/presentation?repository=…&host=…[&sessionId=…&onDemand=true]` | Resolve presence; optionally include exact-session context |

Only the daemon owns writable preference access; preferences survive reset. No FastAPI or Nera data-layer contract changed. There is no separate presentation file or dedicated end-user preference screen. OMP refreshes approximately every five seconds; Claude respects its native cadence. An unreachable daemon can prevent preference resolution and show an unavailable indicator even if `silent` was previously stored.

## Integration evidence and boundaries

- Fresh native, TypeScript, hook and bundled-UI builds passed, as did the web build. The isolated installer smoke packed this checkout into Verdaccio, ran real `npx` setup, exercised hook/HUD/exact-session CLI and unregister restoration, then stopped its daemon and removed the sandbox.
- Final suites passed: 433 Rerail unit tests (50 files), 44 Node integration tests (12 files), 6 real OMP tests (including identical compact/silent enforcement and persisted receipt totals), and 19 affected frontend tests (5 files). Rerail/UI type checks, architecture checks and changed-file ESLint passed. Large UI chunks, stale Browserslist data and Node's test-only local-storage warning remain distinct from failures.
- The inventory-gated measurements recorded before the merge (opaque begin after metadata inventory, deferred content baseline) described a superseded bridge and are withdrawn. Under the merged bridge, opaque tools start immediately and command results wait for a reconciling discovery pass; that wait has not been re-measured against the 30-second one-shot deadline on this checkout. Shell writes made before the baseline reaches their files are reported as uncaptured changes; deletions of never-observed sources remain outside that guarantee.
- Real compiled Claude composition in an isolated Git main/worktree pair selected each workspace's shared command, observed shared edits before/after reinstall, honored genuine local precedence and global fallback, and preserved 1,048,604 predecessor bytes beyond the JSON budget. Five fresh bundled-reader imports measured 19–20 ms and about 58 MiB RSS, versus 178–225 ms/about 130 MiB unbundled after merge; one real compound HTTP read took 96.7 ms and made exactly one request.
- The bundled browser showed exact, missing, wrong-host, missing-host and pending-loading states, with screenshots. URL selection survived Back/Forward. Pointer-click automation timed out, so session activation used the real DOM handler; this proves route behavior, not terminal hyperlink clicking. Loading capture delayed the real sessions response rather than substituting data.

These scratch measurements are not production reliability guarantees or evidence of model compliance. Earlier native OMP/Claude/Codex captures and release caveats remain in the [original experiment history](https://github.com/neralabs/nera-landing/commit/783c13f477fb82032e61c0da5d89df094c44182f) and the PRD; no fresh Pi status-line journey, terminal pointer activation or Codex approval bypass is claimed.
