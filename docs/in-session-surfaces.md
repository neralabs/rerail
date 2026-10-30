# Rerail in-session surfaces

Rerail installs an OMP widget and a Claude Code status line (the HUD) and gives Codex explicit session detail. Product outcomes and acceptance criteria are in the [as-built PRD](in-session-status-line-prd.md).

## Installation and native prerequisites

```sh
npx @neralabs/rerail setup
```

Standard setup registers every surface below. For a source checkout, `just rerail-dev --install-bridge` registers that build through the same registration code. Restart coding agent sessions afterward. No footer flag, extra OMP extension, prototype launcher or presentation settings file is required.

- Every coding agent gets one command set: `/rerail` (what Rerail is doing here, and every command), `/rerail-manage` (open the dashboard), `/rerail-create` (create a rule in your own words), `/rerail-stop`, `/rerail-start`, `/rerail-update`, `/rerail-uninstall` (remove Rerail from this computer; rules and history kept in a backup), `/rerail-help` (the same as `/rerail`) and `/rerail-show-me` (a demo: break a throwaway rule, get steered, fix it, clean up). Settings, status, update checks and reset are CLI commands only (`rerail settings`, `rerail status`, `rerail update check`, `rerail reset`), and so are audits, with `rerail scan`, which is in [internal testing](../README.md#internal-testing).
- The default OMP and Pi extensions register the commands natively and own one below-editor footer widget and its lifecycle timers; they do not replace the native footer. `/rerail-create` and `/rerail-show-me` hand their instructions to the agent; `/rerail-uninstall` asks with a native selector.
- Claude setup registers hooks, composes `dist/hosts/claude/claude-status.js` with existing status output, and writes the commands to `~/.claude/commands/` with exact `allowed-tools` grants. `/rerail-start` runs `rerail start --relay` and also grants set-up's answer commands (`rerail setup --relay …`, through the launcher), so in a repository that is not set up the coding agent relays each question and runs the command printed for the answer without Claude Code asking again. `--relay` keeps set-up in the coding agent's lane even where it runs commands in a terminal. The uninstall removal (`rerail install --remove`) is not pre-approved, so Claude Code asks before it runs. Native idle refresh defaults to five seconds only when the user has no cadence; unregister removes Rerail's added cadence.
- Cursor setup writes the same commands to `~/.cursor/commands/`.
- Codex setup writes one skill per command under `~/.codex/skills/<name>/SKILL.md`, invoked as `$rerail`, `$rerail-manage`, and so on. Registration removes the `/rerail-session` command file an earlier version wrote, and the `rerail-ui` and `rerail-rage-quit` command files and skills from before they were renamed, when they are Rerail's own. Native hook trust and local-execution approval still apply. Rerail does not install the coding agents themselves.
- SQLite's native binding must match the selected Node ABI. Rerail checks and rebuilds it before starting the daemon when necessary; this is a setup prerequisite, not another presentation store.
- Pi's direct packages are aligned to 0.85.1, and its existing integration harness uses `ModelRuntime` with in-memory credentials rather than removed `AuthStorage`.
- Repository discovery remains background work so OMP readiness and path-aware tools never wait for it, and the OMP and Pi attach answers before the session's code-rule evaluator has opened; tool calls, their results and turn ends do not wait for the evaluator either (below). Opaque/pathless tools also start immediately; a command tool (`bash`, `shell`, `exec`, …) is noted at `tool_call`, and its `tool_result`, even a failed one, waits for a discovery pass that reconciles what the shell wrote. Passes are anchored to a filesystem-clock reading taken before the walk, so a source stamped at or after session start is reported as an uncaptured change rather than silently baselined; a deletion is reported only for a source the baseline had already observed.
- A command result or turn end waits at most 10 seconds for that pass. A pass still running then goes on: what it finds is audited and delivered at a later boundary, and the session's history records a `source-discovery-pending` event for the boundary (and again for a close that abandons the pass), which the dashboard timeline shows as shell edits not yet checked. A walk that failed or was cancelled reports a `source-incomplete` failure instead of partial coverage.
- A tool call, its result or a turn end reached while the session's evaluator is still opening (indexing: Git discovery and the content-store attach of a large checkout) does not wait for it. The boundary is answered at once without code rules, and the session's history records a `code-rules-pending` event for it, which the dashboard timeline shows as code rules not yet running. Its edit is not checked later at its result either; the evaluator's first walk still checks a file changed while it ran, as it does any change it did not see made.
- Session events an OMP or Pi extension cannot hand to the daemon are recorded as a gap: a `rerail.events-dropped` session entry, which gets the gap's final count when the gap ends or the session closes or is left, an `events-dropped` session event once the daemon answers again (retried after every later successful request, and tried once more as the session closes), and, unless the presentation is `silent`, a footer line while events are being dropped.

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

In a repository where Rerail is started, each session begins with a short note to the agent: Rerail is on and stays silent until a rule is broken, what a steer looks like, that a steer is advice it may decline by saying why in one line (design N5), to answer "why?" from the steer (N6), to ask the person before turning a rule off or changing its mode and never to turn a rule off or stop Rerail to get past one (N7, N9), the session's dashboard link, and that `/rerail` (Codex: `$rerail`) lists the commands. In Cursor it also asks the agent to relay each steer's line, since Cursor shows the person nothing from a hook. Claude Code and Codex receive it as `SessionStart` `additionalContext`, Cursor as `sessionStart` `additional_context`, and OMP and Pi as a hidden `before_agent_start` message (`customType: "rerail.session-note"`). Rerail never writes instruction files. The agent's replies are its own: the note and the steer block guide them, nothing enforces them.

### `/rerail`

`/rerail` is rendered by `rerail session [<host> [<session-id>]]`:

```text
● Rerail is on in this repo · acme-web
  26 rules · steered 1× in this session
  See what it logged → http://127.0.0.1:4737/?sessionId=…&host=claude-code#sessions

Commands
/rerail-manage       open the dashboard
…
Or just ask me, in plain words.

In a terminal: rerail status · rerail version · rerail update · rerail daemon stop
```

### Steer

Each delivery is one block, which only the coding agent reads. It is written for the agent alone: what is wrong, where, what to do, and how much latitude it has. None of Rerail's own vocabulary (state glyphs, modes, rule packs, links) is in it, and nothing parses it.

```text
<lead>
- <path>:<line>:<col> — <message> [<rule id>]
  Suggested fix: replace 14:3–14:12 with `logger.info(user)`
- <path> — <message> [<rule id>]
- <message> [<rule id>]
<latitude>
```

Findings are listed in the order they were found, each on its own line: `path:line:col` (or the path alone for a finding about a whole file, nothing for one with no path), the message, then the rule id in brackets. A multi-line message keeps its later lines indented under its finding. A suggested fix follows its finding: `Suggested fix: delete <range>`, `Suggested fix: replace <range> with <inline code>` (a CommonMark backtick fence longer than any run of backticks in the fix), or, for a fix over several lines, `Suggested fix, replacing <range>:` and the fix in a `~~~` fence (longer when the fix holds one). Ranges use an en dash (`3:1–5:2`), and a fix over 240 characters is cut with `… (truncated)`.

The lead says what Rerail did:

| Trigger | Lead |
| --- | --- |
| An edit's check | `Rerail found 1 problem in your last edit:` (`N problems`) |
| A check run again after its sources changed | `Rerail re-checked; still open:` |
| Findings that reach the agent after it moved on (the missed-steer digest; a Claude Code steer held past `Stop`) | `Rerail finished checking your earlier edits after you moved on:` |
| A repository audit | `Rerail checked the repository and found 1 problem:` (`N problems`) |
| A tool call or conversation rule | `Rerail has guidance for this <Tool> call:`, or `Rerail has guidance for this step:` when no tool is named |

The latitude closes it: `Fix it, or tell the user in one line why you won't.` (`Fix these, …` for more than one finding; `Follow it` and `Follow these` for a tool call's guidance). A Block-mode rule steers until blocking ships, so its block reads the same. A Log never reaches a session; where one is recorded its block ends `Recorded only.`

The person's line is not read out of the block. Each delivery carries its rules (`rules`: id, state and where the rule comes from, in first-seen order) beside `ruleIds`, and each coding agent's adapter builds the lines from them, linking each rule to its page on the dashboard. Rules are kept wherever a delivery is held (Claude Code's pending file, the session ledger), so a steer that goes out later still has its lines. The glyphs are the converged set: `○ logged`, `◆ steered`; a Log never reaches a session, so sessions show `◆` only.

The person sees one line per rule, `◆ Rerail · <rule id>  steered  ·  why?`, with the link on `why?` wherever the coding agent can draw one. Where it draws colour (Claude Code, OMP) Rerail is the footer's mark, the coloured `R E R A I L` (plain letters under `NO_COLOR`); Codex and Cursor show the word:

| Coding agent | Line | When a steer is found as the turn ends |
| --- | --- | --- |
| Claude Code | `systemMessage`, under the tool call after Claude's `PostToolUse:<Tool> says:`; `why?` is an OSC 8 link in a terminal known to draw them (OMP's list — kitty, Ghostty, WezTerm, iTerm2, VS Code, Alacritty — plus Orca, also inside tmux, and Windows Terminal, through tmux from 3.4), else `why? → <url>`. `RERAIL_NO_HYPERLINKS=1` / `RERAIL_FORCE_HYPERLINKS=1` (or OMP's `PI_*` pair) decide instead. Two or more lines start on their own line. | `Stop` shows the line only; the block reaches the agent at the next tool call (or session start), without the line again, led as a late steer (`Rerail finished checking your earlier edits after you moved on:`). Never with the person's next prompt: `UserPromptSubmit` context rides inside the person's own turn. A `Stop` hold would show the person the whole block as `Stop hook error:` (verified on 2.1.296). |
| Codex | `systemMessage` (`↳ Hook · …`), plain text: `… steered  ·  why? → <url>`. | `Stop` shows the line only; the block reaches the agent with the next hook that carries context (the person's next prompt, a tool call, a session start), as a developer message, led as a late steer. A `Stop` hold (`decision: "block"`) would hand it to Codex as a user-role `<hook_prompt>` that compaction keeps. |
| Cursor | None of Rerail's own. The agent is asked to begin its reply with `… steered  ·  [why?](<url>)`. | Waits for the next `postToolUse` or session start. With the experimental missed-steer digest on, `stop` answers with a `followup_message` carrying the waiting steers, which Cursor submits as the next message, while its `loop_count` is under 3. |
| OMP | Rerail's message renderer draws the same line from the rules in the message's `details`, in OMP's theme colours, `why?` an OSC 8 link where the terminal draws links; Ctrl+O adds where each rule comes from and the block, dimmed. A steer sent as a message is a `rerail.rule` message. A steer that rides an edit's tool result (see Steers and compaction), which OMP never draws, gets a `rerail.steer-line` message for the person, sent as an aside, so OMP shows it right after that call's results and before the next model call without interrupting the call's batch. Rerail's `context` handler takes it out of every model request, and it records no receipt, so ` · steered N×` counts the steer once. Pi keeps its default card, which shows the block: its extension API gives Rerail no components to draw with. | The end of the run (`session_stop`) waits for the turn's own check, up to 10 seconds, then asks the daemon for the run's yield (`yield`): the steers still pending (and, with the experimental missed-steer digest on, the digest below), sent as steers so OMP continues the run with them before you read "done". Pi does the same from `agent_before_settle`, and continues. A steer that only arrives after the session is idle waits for the next message. |

After an edit, the Claude Code, Codex and Cursor hooks wait up to one second for that edit's rule check before they answer, so a steer lands under the edit and the coding agent can act on it in the same turn; a check usually takes tens of milliseconds. OMP and Pi wait up to 10 seconds, at a tool result and at a turn end: an edit's check is done long before, while a shell command that wrote thousands of sources does not stall the agent for the minutes their checks take. While checks a boundary stopped waiting for are still draining, no later boundary waits for them again. A steer whose check outlasts its boundary's wait goes out at the next hook, reply or poll. The caps are global (`HOOK_EDIT_CHECK_MS` and `NATIVE_EDIT_CHECK_MS` in `src/runtime/runtime.ts`), not set per rule.

**Steers and compaction.** A steer never becomes the person's message, before or after the coding agent compacts its context:

| Coding agent | How |
| --- | --- |
| OMP, Pi | A steer from an edit's own check goes into that edit's tool result, after its output (rolled back with the experimental `rewind`: in place of it, marked an error), not into a message. A steer found later, the session-start note and OMP's `rerail.steer-line` for the person are `rerail.*` custom messages, which both count as the start of a turn and Pi shows the summarizer as `[User]`: Rerail's `session_before_compact` handler folds a split turn such a message leads back into the history, so the summary keeps the person's own request rather than an "Original Request" read off the steer, and leads every Rerail message the summarizer reads with `[Rerail: automated rule check sent to the agent, not a message from the user]`. It leaves steer lines out of what is summarized, since the summarizer reads their steers in the tool results; one in OMP's kept tail stays there, labelled, because OMP pairs that tail with the session entries it keeps. OMP skips its speculative compaction while any extension handles `session_before_compact`, so Rerail registers its handler only once it has attached in a started repository. |
| Claude Code | `additionalContext` beside a tool result or at session start, never at `UserPromptSubmit`. |
| Codex | `additionalContext`, a developer message (local compaction drops it), never a `Stop` reason. |
| Cursor | `additional_context` at `postToolUse` and session start only; `beforeSubmitPrompt` never carries a steer. The experimental missed-steer digest is the exception: its `followup_message` is submitted as the next message. |

**Experimental: missed-steer digest.** Off unless `RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST=1` is set in the environment of the process that runs Rerail's runtime (the daemon for OMP, Claude Code, Codex and Cursor; the coding agent itself for a Pi session running Rerail in process) and, for Cursor's `stop` follow-up, of Cursor's hook. Its behaviour may change or go away. With it on, a steer whose check outlasts its boundary's wait is not handed to some later, unrelated step: its findings are held, and at the agent's yield (the end of its run, in the column above) they reach it as one steer led by `Rerail finished checking your earlier edits after you moved on:`, listing every finding, each once, in the order they arrived; a file edited again meanwhile drops out of it. Findings that arrive after a yield wait for the session's next one. The digest has no size limit yet.

**Experimental: steer strategies.** Off unless `RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1` is set in the environment of the process that runs Rerail's runtime (the daemon for OMP, Claude Code, Codex and Cursor; the coding agent itself for a Pi session running Rerail in process). Its behaviour may change or go away. Off, every code-rule steer is a `message`, which is the default: with the default `ai-output` scope, a steer about the lines the edit added or changed, sent as its own message. With it on, the checkout's steer strategy (`steerStrategy` in its audit settings, `rerail settings --steer-strategy`) decides how a code-rule steer reaches the agent:

| Strategy | What the agent gets |
| --- | --- |
| `message` (default) | The steer as above: in OMP and Pi in the edit's tool result when the edit's own check found it, else a `rerail.rule` message (see Steers and compaction). |
| `rewind` | When the edit's own check steers before its tool result is answered, Rerail puts every file the call captured back as it was before the edit (a file it created is deleted), all or none, and the steer opens with what it put back and that the edit is to be redone following the rule. OMP and Pi put it into that tool call's result, marked as an error, in place of a message; Claude Code, Codex and Cursor carry it on the same `PostToolUse`/`postToolUse` context as above. A steer found later, or for an edit Rerail cannot restore (a preimage it could not read, a file changed since, a file another unfinished tool call also edits, a restore that fails), goes out as `message`. |
| `subagent` | The steer as above, ending with a note that it comes from Rerail, that the agent's task is unchanged, and to hand the fix to a subagent: `rerail_fix` in Pi (Rerail registers it with the first such steer; it runs a child `pi -p --no-session --no-extensions --tools read,edit,write,grep,find,ls` with the session's model, so the child loads no extensions, neither Rerail nor the user's, and has Pi's file tools but no shell; a child still running after 10 minutes is stopped and the fix fails; it returns the child's answer and usage), `task` in OMP, `Task` in Claude Code and Cursor, `spawn_agent` in Codex. |

`benchmark/steer-strategy.ts` measures the three on real harnesses.

A hook never fails a tool call. Only the hook that starts a session may start the daemon; any other hook that finds none running answers `{}` at once (one that finds another package's daemon, after an update, replaces it as before), and every hook exits 0, so a daemon that is down is invisible in the chat instead of a `hook error` under every tool call. In OMP and Pi the tool hooks catch their own failures and let a tool go after 25 seconds, before the host's 30-second deadline would block it. The footer says `not running`; Codex, which has none, gets `Rerail ✕ not running · start it with: rerail daemon start` once, when a session starts and no daemon can be reached.

The runtime records no link. Each coding agent's adapter links each of a delivery's rules as it hands the steer over (`linkDeliveries`), using the same dashboard the footer links to: the Rerail daemon's recorded URL when it is loopback, scoped to the session's repository and checkout. The `rerail events` stream does the same for its harness, using the session's cwd (else the stream's working directory), and returns each rule's `url` with its delivery. A daemon bound to `0.0.0.0`, or no daemon at all, leaves the rules unlinked, and the person's line carries no `why?`, rather than pointing a browser at an address it cannot open. The session-start note and `rerail rules add` resolve their links the same way.

Primary references: [Claude status lines](https://code.claude.com/docs/en/statusline), [skills](https://code.claude.com/docs/en/skills), [settings/worktree precedence](https://code.claude.com/docs/en/settings#where-claude-code-keeps-the-local-file-in-a-git-repository), [Codex configuration](https://developers.openai.com/codex/config-reference), and [inspected Codex status-line source](https://github.com/openai/codex/blob/a1294e57f12e9474088a2f6e6a9142ed3e1ddccd/codex-rs/tui/src/bottom_pane/status_line_setup.rs). OMP source inspection covered extension types, the extension UI controller and status-line segments of the OMP version pinned in `package-lock.json`.

## What the footer means

```text
R E R A I L ● 26 rules · steered 3×
```

` · steered N×` appears only above zero. Other states: `R E R A I L ○ not on here` where Rerail is not started or was stopped, `R E R A I L ○ not set up · rerail start` in a started repository that is not set up, `R E R A I L ✕ not running` when no daemon answers, and `R E R A I L ● session unavailable` before the session is recorded. `engine unavailable` and `audit failed (N failures)` are appended when relevant; N counts every failure of the latest audit, those its list of at most 20 leaves out included. A completed audit with coverage gaps appends `N files not checked`, `N rules ran out of work` and `N rules stopped by file work` (the file's shared work or its work ceiling stopped them) instead. The footer is static: it refreshes every five seconds and never animates. Claude Code (status line), OMP and Pi (below-editor widget) show it; Codex and Cursor have no footer, and `/rerail` (`$rerail`) is the way in.

In OMP, ` · indexing` is appended while the daemon indexes the repository and opens the session's code rules (shown only when that takes over half a second). Nothing waits for it: until it ends, the session's tool calls, results and turn ends go on without code rules (see the boundaries above). Pi does not show it.

- **Rules** is configured session coverage: the effective conversation-rule registry and enabled compiled code-rule catalog, including host options/files, database overrides and session mutations. It is not all enabled database rows, rules matching the current file, or successful checks.
- **Steered** counts the rules named in successful native `sendMessage` steers or successfully emitted hook responses with nonblank guidance: one per line the person saw, so two rules in one edit count 2 (a delivery that names no rule counts 1). A Claude Code steer found as the turn ends counts when its line shows at `Stop`, and not again when its block reaches the agent with the next message. Logs, matches, queued/deferred guidance and failed sends do not count. Receipt retries deduplicate; a new actual steer counts again, against its originating session.
- **Unknown is not zero.** Older sessions and histories lost in a reset cannot establish complete totals and show `steering unavailable`. Invalidated coverage shows `rules unavailable` until the host synchronizes. Routine streaming deltas do not recompute coverage.
- A steer proves neither model consumption nor correction/recheck success. A crash or unrecoverable receipt-write failure after delivery can undercount: host delivery and the SQLite write are not atomic.
- Engine/session unavailability and audit failures stay explicit. Violations are not an outstanding-work queue or evidence that changes are ready for human review. Explicit detail retains health, scope/status, violation/failure totals and `not evaluated` when no run exists.

`R E R A I L` and the steering count link to the exact current session; the rule count links to `/#rules` on the same daemon. The six letters are Rerail's mark without its ray (`RERAIL_MARK_RAMP` in `src/presentation/rerail-mark.ts`, from the session enforcement design): each letter its own colour from `#A5ADFB` to `#ECDAE4`, in 24-bit where `COLORTERM` is `truecolor` or `24bit`, else the nearest of 256 colours, and uncoloured under `NO_COLOR`. The state mark takes the state's colour (`●` green, `○` dim, `✕` red); nothing else is coloured.

Presentation reads are bounded and passive: one HTTP request validates daemon identity, resolves presence and returns exact-session HUD counts with a bounded latest-evaluation summary. They never start/retarget a daemon, trigger evaluation or open a writable database. Full audit bodies and rule lists belong to explicit dashboard reads. The installed passive reader is tree-shaken at build time while retaining schema-derived validation.

## Session links and receiving route

```text
http://127.0.0.1:<daemon-port>/?sessionId=<encoded-id>&host=<encoded-host>#sessions
```

The reader discovers the daemon for the originating database, validates loopback origin and canonical database, then verifies daemon PID/database and the session ID, host and working directory. Redirects are rejected and reads are bounded. Locator state is re-read on refresh so daemon restarts need not leave stale links.

The existing browser route uses the URL for both view and selection, including reloads and Back/Forward. Missing sessions, missing hosts and host mismatches show unavailability, never an arbitrary/latest session. Selecting a recorded session writes both ID and host. Rules, settings and diagnostics navigation remain intact; legacy `#scans` resolves to diagnostics.

Every session records into the Rerail home's one database, so the daemon answers status for sessions of any repository. Its Rules page and listings belong to the repository it was started in; `rerail manage` from another repository reports `daemon serves another repository` rather than linking to the wrong Rules page. Bookmarks are not durable cross-database identities or synchronized settings. A recorded `live` label means no end timestamp; it is not proof that a coding-agent process is still running.

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

- Commands run by a command tool wait up to 10 seconds for a reconciling discovery pass before their results are evaluated; a pass that runs longer reports its findings at a later boundary and records `source-discovery-pending` in the session's history. Shell writes made before the baseline reaches their files are reported as uncaptured changes; deletions of never-observed sources remain outside that guarantee.
- Claude status composition selects each workspace's shared command in a Git main/worktree pair, observes shared edits without reinstalling, honours genuine local precedence and global fallback, and preserves predecessor commands larger than the JSON budget.
- Session links prove route behavior in the bundled browser (exact, missing, wrong-host, missing-host and loading states, Back/Forward); terminal hyperlink activation depends on the terminal.
- None of this is a production reliability guarantee or evidence of model compliance. No Pi status-line journey, terminal pointer activation or Codex approval bypass is claimed.
