# Rerail in-session surfaces

In a coding-agent session, Rerail shows a note to the agent at start, a steer when a rule is broken, a footer with counts, and `/rerail` on request.

| Coding agent | Footer | On request | Session identity |
| --- | --- | --- | --- |
| Claude Code | Status line, beside yours | `/rerail` | Session id, working directory |
| OMP | Widget below the editor; native footer kept | `/rerail` | Session id, working directory |
| Pi | Widget below the editor; native footer kept | `/rerail` | Session id, working directory |
| Codex | None (no slot) | `$rerail` | Thread id, never the latest session in a directory |
| Cursor | None | `/rerail` | None: Cursor cannot name its session, so `/rerail` is repository-level |

## Setup

One command registers every surface; restart your coding-agent sessions afterward.

```sh
npx @neralabs/rerail setup
```

Nothing else needs turning on. Rerail does not install the coding agents.

### Commands

Every coding agent gets the same commands:

- `/rerail` or `/rerail-help`: what Rerail is doing here, and every command.
- `/rerail-manage`: open the dashboard.
- `/rerail-create`: create a rule in your own words, or change one, where it applies included.
- `/rerail-start`, `/rerail-stop`, `/rerail-update`.
- `/rerail-uninstall`: remove Rerail from this computer; rules and history are kept in a backup.
- `/rerail-show-me`: a demo that breaks a throwaway rule, gets steered, fixes it and cleans up.
- `/rerail-bug-report`: fix your own rule, or report a Rerail bug in an issue.

Terminal only: `rerail settings`, `rerail status`, `rerail update check`, `rerail reset`, and audits (`rerail scan`, in [internal testing](../README.md#internal-testing)).

<details>
<summary>How each coding agent registers them</summary>

- **OMP and Pi:** native extension commands. `/rerail-create` and `/rerail-show-me` hand instructions to the agent; `/rerail-uninstall` asks you with a selector.
- **Claude Code:** setup adds hooks, adds Rerail to your status line, and writes commands to `~/.claude/commands/` with their tool permissions. `/rerail-start` may run set-up's commands without re-asking, so in a repository not set up the agent relays each set-up question to you. `/rerail-uninstall` is not pre-approved; Claude Code asks first.
- **Codex:** one skill per command in `~/.codex/skills/<name>/SKILL.md`, invoked as `$rerail`, `$rerail-manage`, and so on. Codex's hook trust and command approval still apply; it may ask before `$rerail` reads the local daemon.
- **Cursor:** the same commands in `~/.cursor/commands/`.

</details>

### Claude Code status line

Rerail appends to your status-line command instead of replacing it; `rerail install --remove` restores the old one.

<details>
<summary>Details</summary>

- This includes a command a Git worktree inherits from its main checkout; later edits to it apply without reinstalling.
- It refreshes every five seconds unless you set your own interval.
- Re-running setup changes nothing. `rerail install --remove` restores your previous status line and interval and leaves other settings alone.
- What Rerail replaced is recorded in `~/.rerail/harness.sqlite`.

</details>

## Session-start note

Where Rerail is started, each session opens with a short note to the agent that you do not see.

It explains steers and asks the agent to check with you before turning a rule off. Rerail never writes instruction files.

<details>
<summary>What the note says</summary>

- Rerail is on, silent until a rule is broken, and what a steer looks like.
- A steer is advice: fix it, or tell you in one line why not, and carry on.
- Answer "why?" from the steer: the rule id and what it matched.
- Ask before turning a rule off or changing its mode; never turn a rule off or stop Rerail on its own to get past one.
- The session's dashboard link, and that `/rerail` (Codex: `$rerail`) lists the commands.

Cursor shows you nothing from a hook, so there the note also asks the agent to begin its reply with each steer's line.

The note and steers guide the agent; nothing enforces its replies.

</details>

## `/rerail`

`/rerail` (Codex: `$rerail`; terminal: `rerail session [<host> [<session-id>]]`) shows what Rerail is doing in this session, then every command.

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

- **Not started:** says so and points at `/rerail-start`.
- **No daemon:** says `✕ Rerail is not running.` and how to start it.
- **Daemon not answering:** the daemon runs but gave no answer within 2 seconds, so it says to ask again, and to run `rerail status` if that keeps happening, rather than how to start it. It does not wait on the daemon a second time for the dashboard link.
- **Cursor:** shows the repository's rule count and a link to its rules.

## Steer

A steer has two parts: a block only the coding agent reads, and one line per rule for you.

### What the agent reads

```text
<lead>
- <path>:<line>:<col> — <message> [<rule id>]
  Suggested fix: replace 14:3–14:12 with `logger.info(user)`
- <path> — <message> [<rule id>]
- <message> [<rule id>]
<latitude>
```

A lead says what Rerail did. Each finding gives its place, message and rule id, sometimes a suggested fix.

<details>
<summary>Finding format and limits</summary>

Findings appear in the order found. The place is `path:line:col`, the bare path for a whole-file finding, or omitted without a path.

A suggested fix follows its finding: `Suggested fix: delete <range>`, `Suggested fix: replace <range> with <code>`, or, over several lines, `Suggested fix, replacing <range>:` and a `~~~` fence. Ranges use an en dash (`3:1–5:2`). Fixes over 240 characters end in `… (truncated)`.

The block has no Rerail vocabulary: no glyphs, modes, packs or links.

Repository text (paths, captured code, fixes) is cleaned: control and invisible characters go, and line breaks inside a line become spaces. Parts are capped and marked `…`: places, rule ids and tool names at 300 characters, message lines at 600, a whole message at 2,000, and snippets at their first line and 80 characters.

</details>

<details>
<summary>Leads by trigger</summary>

| Trigger | Lead |
| --- | --- |
| An edit's check | `Rerail found 1 problem in your last edit:` (`N problems`) |
| A check re-run after its sources changed | `Rerail re-checked; still open:` |
| Findings arriving after the agent moved on | `Rerail finished checking your earlier edits after you moved on:` |
| A repository audit | `Rerail checked the repository and found 1 problem:` (`N problems`) |
| A tool call or conversation rule | `Rerail has guidance for this <Tool> call:`, or `Rerail has guidance for this step:` with no tool named |

</details>

The latitude closes it: ``Fix it, or tell the user in one line why you won't and run `rerail feedback <rule-id> '<that line>'`.`` On Windows the command pipes the line in instead, which cmd.exe never parses: ``echo '<that line>' | rerail feedback <rule-id> -`` in Git Bash, and for Codex and Cursor, which run PowerShell there, three lines: ``$OutputEncoding = [Text.UTF8Encoding]::new(); @'``, a space and the line, then ``'@ | rerail feedback <rule-id> -``. PowerShell reads nothing inside that here-string, so no quote in the line can end it and run the rest as a command, and the first line has Windows PowerShell 5.1 pipe UTF-8, where it otherwise pipes every character outside ASCII as a `?`. `rerail feedback` keeps the reason on this computer ([Steer feedback](reference.md#steer-feedback)).

<details>
<summary>Latitude variants and modes</summary>

- Several findings read `Fix these, …`; a tool call's guidance reads `Follow it` or `Follow these`.
- The command names the rule when there is one; with several, the agent fills in `<rule-id>`.

Block-mode rules steer the same way; blocking is not built yet. Log-mode rules never reach a session.

</details>

### What you see

One line per rule, `◆ Rerail · <rule id>  steered  ·  why?`, where `why?` opens the rule on your dashboard.

<details>
<summary>Mark, link and glyphs</summary>

- **Mark:** the footer's coloured `R E R A I L` where the agent draws colour (Claude Code, OMP); the word in Codex and Cursor.
- **Link:** `why?` appears only while a daemon answers on a loopback address. With none, or one bound to `0.0.0.0`, the line has no link rather than a dead one.
- **Glyphs:** `◆ steered`, `○ logged`. Log-mode rules never reach a session, so sessions show `◆` only.

</details>

<details>
<summary>Per-agent placement and turn-end steers</summary>

Where your line appears, and what happens to a steer found as the turn ends:

- **Claude Code:** under the tool call, after `PostToolUse:<Tool> says:`; two or more lines start on their own line. `why?` is a link in terminals known to draw them ([below](#links-in-the-terminal)), else `why? → <url>`. A turn-end steer goes out at the turn's end: its line for you, and its block for the agent as `Stop hook feedback`, which continues the turn so the agent can act on it. Once a Stop hook, Rerail's or another, has kept the turn going, a steer found when it ends again waits, line and block together, for the agent's next tool call or session start, led as a late steer, so Rerail never keeps a turn going twice.
- **Codex:** under the tool call (`↳ Hook · …`), plain: `… steered  ·  why? → <url>`. Codex takes a turn-end block as your message, so a turn-end steer shows nothing then: its line and its block ride the next hook carrying context (your next prompt, a tool call, a session start), the block led as a late steer.
- **Cursor:** no line of Rerail's own; the agent is asked to begin its reply with `… steered  ·  [why?](<url>)`. A turn-end steer waits for the next tool result or session start.
- **OMP:** drawn in OMP's theme colours, `why?` a link where the terminal draws them. Ctrl+O expands it to show each rule's source and the block, dimmed. A steer riding an edit's tool result shows right after that call's results, before the next model call; the agent never sees the line. At turn end, the run waits up to 10 seconds for the turn's check, then pending steers continue the run before it reports done. A steer arriving once the session is idle waits for your next message.
- **Pi:** Pi's default message card, showing the whole block; Pi's extension API gives Rerail nothing to draw its own line with. Turn end as OMP.

</details>

### When a steer arrives

Each coding agent waits briefly after an edit for its check, so the steer lands under that edit. A steer never becomes your message.

<details>
<summary>Wait times</summary>

- **Claude Code, Codex, Cursor** wait up to one second after an edit for its check, so the agent sees the steer under the edit and can act that turn. Checks usually take tens of milliseconds.
- **OMP and Pi** wait up to 10 seconds at a tool result and at turn end, so a shell command that wrote thousands of files does not stall the agent for minutes.

Slower checks steer at the next hook, reply or poll. The waits are fixed, not per rule.

</details>

<details>
<summary>How each agent carries a steer</summary>

A steer never becomes your message, before or after the agent compacts its context:

- **OMP and Pi:** a steer from an edit's own check goes into that tool result. Later ones are Rerail messages, which compaction summaries label automated rule checks, not your messages. Pi hands the model such a message as one of yours, so in Pi it opens with a line saying it is an automated note from Rerail, advice rather than a new task, and that the agent carries on with your request.
- **Claude Code and Cursor:** context beside a tool result, at session start or, in Claude Code, at the turn's end; never with your prompt.
- **Codex:** a developer message, which can arrive with your next prompt but is not part of it.

</details>

[Experimental steer features](internal-testing.md#experimental-steer-features) (missed-steer digest, `rewind` and `subagent` strategies) are off unless you turn them on.

### Edits made by shell commands and before rules are ready

Rerail checks shell-written files when the command finishes. Edits made while a large repository indexes, or while a session idle for 30 minutes opens again, are checked in the first pass after.

<details>
<summary>Shell edits and indexing</summary>

- A shell result, or a turn end, waits up to 10 seconds for Rerail to find what the shell wrote. Later finds are steered at a later step, and the dashboard timeline says rules had not yet seen shell edits there.
- A file the shell deleted is reported only if Rerail had seen it.

In a large repository, Rerail indexes at session start before code rules can run; nothing waits for it.

- Edits made meanwhile are not checked at their own result; the first pass after indexing checks every changed file.
- OMP's footer shows ` · indexing` while it lasts, only if it takes over half a second.
- The dashboard timeline says code rules were not yet running at those steps.

The daemon closes a session that sends nothing for 30 minutes, and frees a checkout's compiled rules 15 minutes after
its last session ends. The session's next event opens it again, and its rules may compile again (about 12 s of CPU for
a TypeScript repository's five packs on a Windows laptop). In Claude Code, Codex and Cursor, a reopened session's tool
calls and turn end wait up to 20 seconds for its rules, so its first edit is checked and steered at its own result as
every later one is. A daemon restart or a resumed session reopens the same way. Rules still not open after 20 seconds
are not waited for further: that edit is checked by the first pass once they open.

</details>

### Files Git ignores

While the repository follows `.gitignore` (the default), files Git ignores are not checked. The session's record keeps a tool edit Git's rules left unchecked, and a change that can make those rules ignore more; the dashboard timeline counts both. Some skips are not recorded (below).

<details>
<summary>What is recorded, and what is not</summary>

Recorded:

- A tool edit (Write, Edit, `apply_patch`) of a file a rule would read, which Git ignores: the file, and the ignore file, line and pattern that ignore it, as `git check-ignore -v` names them (`ignored-edit`).
- A change that can make Git's ignore rules ignore more (`ignore-rules-changed`), with the patterns it added and the negations (`!pattern`) it removed (`ignoresMore`). In a file without a negation, comments, blank lines, order and a pattern the file already held are no change, nor is a pattern removed. In a file with a negation, Git's last matching pattern wins, so any other change to its patterns, a reorder or a pattern repeated after a negation, is recorded too (`reordered: true`).
  - `via: "tool"`: a `.gitignore`, `.git/info/exclude` or `core.excludesFile` that a finished tool edit wrote, compared with what it held before the edit.
  - `via: "observed"`: a `.gitignore` the session's walk finds new or changed after a command or at a turn end, even one Git ignores (a `*` in it ignores the file itself, so no diff shows it); and `.git/info/exclude` or `core.excludesFile`, compared at each command and turn end. The exclude files are those of the session's checkout and of each checkout nested in it that runs rules of its own (a submodule, a recorded repository).
  - `git config core.excludesFile <file>` is followed: the repository's `config` and `config.worktree` and the global gitconfig are watched, and once one changes, an excludes file it newly names is compared from empty.
  - A `.gitignore` the session's first walk finds already written after the session started is compared from empty, not taken as the baseline.
  - It is what a look saw, whoever made it: every session open in the repository records a change to `.git/info/exclude` at its next look, and so does a `.gitignore` that `git pull` or `git checkout` brought; a change made while no session is open is on no record ([#484](https://github.com/neralabs/rerail/issues/484)).
- `sessions_get` counts both in `gaps` (`ignoredEdits`, `ignoreRuleChanges`); `session_events` has each one, withheld from coding agents since they quote hidden ignore files.

Not recorded:

- A shell write inside an ignored folder, file by file: the walk never enters such a folder, which is what keeps it fast. The change to the ignore rules that put the folder out of reach is recorded, if a session saw it.
- Anything under the built-in folders Rerail never reads (`.git`, `node_modules`, `dist`, `build`, `vendor` and the rest of `IGNORED_DIRECTORIES`), whatever the setting.
- A `.gitignore` above the session's folder, when the session starts below its checkout's root: the walk starts at the session's folder.
- An excludes file named by the system gitconfig, which only an administrator writes, or by a file an `include.path` pulls in.

</details>

### When the daemon is down

A hook never fails a tool call; the footer says `not running`.

<details>
<summary>Behaviour without a daemon</summary>

- Only a session start may start the daemon. Other hooks that find none let the call go at once, so you get no `hook error` under every call.
- In OMP and Pi, a Rerail tool hook releases the tool after 25 seconds regardless.
- Codex, with no footer, shows `Rerail ✕ not running · start it with: rerail daemon start` once at session start.

Events the OMP or Pi extension cannot hand to the daemon are recorded as a gap, not lost silently. Meanwhile the footer adds `Rerail: N session events dropped — daemon not answering` (not under `silent`), and the dashboard timeline counts the events that never arrived.

</details>

## What the footer means

The footer shows the session's rule count, steers and any problem state.

```text
R E R A I L ● 26 rules · steered 3×
```

` · steered N×` appears only above zero. Other states:

| Footer | Meaning |
| --- | --- |
| `R E R A I L ○ not set up · rerail start` | Started, but no rules set up |
| `R E R A I L ✕ not running` | No daemon answers as Rerail: none is recorded, its process is gone, or it refused the read |
| `R E R A I L ● not answering` | The daemon is running but gave no answer within 2 seconds, as when it is busy |
| `R E R A I L ● starting` | Session not recorded yet: Rerail is still starting it |
| `R E R A I L ● session unavailable` | The session could not be read |

Where Rerail is not started, or was stopped, there is no footer; `/rerail` says `not on here`.

It refreshes every five seconds and only reads: it never starts a daemon, runs a check or writes anything.

<details>
<summary>Suffixes appended when they apply</summary>

- ` · worktree` after the rule count, while the checkout runs its own rules from `.rerail/rules` ([a worktree's own rules](rule-packs.md#a-worktrees-own-rules)). Links to their pack page: `R E R A I L ● 27 rules · worktree · steered 3×`.
- ` · N to approve` while N rules or packs on here run in Log awaiting [approval](rule-scoping.md#approval); a pack counts once. Links to the repository's Overview, which lists them.
- ` · indexing` in OMP, not Pi, during indexing ([above](#edits-made-by-shell-commands-and-before-rules-are-ready)).
- `engine unavailable` if the rule engine could not load.
- `audit failed (N failures)` if the latest audit failed; N counts every failure, including any beyond its list of at most 20.
- For a completed audit with gaps: `N files not checked`, `N rules ran out of work`, `N rules stopped by file work`.

The footer never animates. Full audits and rule lists are on the dashboard.

</details>

### What the numbers mean

**Rules** is the session's configured coverage. **Steered** counts steer lines you saw. Unknown is shown as unavailable, never as zero.

<details>
<summary>Exact counting</summary>

- **Rules** is the session's configured coverage: code and conversation rules that are on, including mid-session changes. Not rules matching the current file, nor checks that passed.
- **Steered** counts lines you saw: two rules in one edit count 2; a delivery naming no rule counts 1. A steer counts once, when its line shows, which is when its block reaches the agent. Logs, matches, queued guidance and failed sends don't count.
- **Unknown is not zero.** Older sessions, and history lost in a reset, show `steering unavailable`; recomputing coverage shows `rules unavailable`.
- **A steer proves nothing** about whether the agent read it or fixed anything. A crash right after sending can leave it uncounted.
- **Violations are not a to-do list** or a review-ready signal. Rerail running normally and an audit reporting failures are separate states.

</details>

### Links and colour

`R E R A I L` and the steer count link to this session on the dashboard; the rule count links to the rules page.

<details>
<summary>Colours</summary>

- The six letters run from `#A5ADFB` to `#ECDAE4`: 24-bit where `COLORTERM` is `truecolor` or `24bit`, else the nearest of 256 colours, none under `NO_COLOR`.
- The state mark is coloured by state (`●` green, `○` dim, `✕` red); nothing else is.

</details>

### Links in the terminal

Steer lines in Claude Code and OMP are clickable (OSC 8) links in terminals known to draw them; elsewhere the URL is printed.

<details>
<summary>Supported terminals and overrides</summary>

- **Supported:** kitty, Ghostty, WezTerm, iTerm2, VS Code, Alacritty, Windows Terminal, Orca, and tmux 3.4 or later; never GNU screen.
- **Override:** `RERAIL_NO_HYPERLINKS=1` or `RERAIL_FORCE_HYPERLINKS=1` (OMP: `PI_NO_HYPERLINKS` / `PI_FORCE_HYPERLINKS`).

The footer always carries its links; your terminal decides if they are clickable.

</details>

## Session links

A session link opens the dashboard's Sessions view on one session.

```text
http://127.0.0.1:<daemon-port>/?sessionId=<encoded-id>&host=<encoded-host>#sessions
```

- The session stays selected across reloads and Back/Forward.
- A session outside the dashboard's repository gets a notice, and the most recent session shows instead.
- "Live" means no recorded end, not that the coding agent is still running.

## Hiding the footer

The `silent` preference hides the footer and dropped-events line without changing enforcement.

Rules stay on, steers arrive, modes hold, and `/rerail`, `$rerail`, the CLI and the API still answer. Set it through the daemon's API (also an MCP tool); there is no settings screen or file yet.

<details>
<summary>API, precedence and storage</summary>

| Operation | Purpose |
| --- | --- |
| `GET /api/presentation-preferences` | List stored preferences |
| `PUT /api/presentation-preferences` | Set `{ repository, host, presence }` |
| `DELETE /api/presentation-preferences?repository=…&host=…` | Remove an override, revealing the next |
| `GET /api/presentation?repository=…&host=…[&sessionId=…&onDemand=true]` | Resolve the preference, optionally with session counts |

A preference is `compact` (default) or `silent`, keyed by repository and coding agent; empty means all. The most specific wins: repository and agent, repository, agent, database default, then `compact`. Removing an override reveals the next.

Preferences live in the Rerail database and survive `rerail reset`.

- `repository` is an absolute path; symlinked spellings (`/tmp`, `/private/tmp`) match.
- Changes show at the footer's next refresh.
- Preferences are read through the daemon, so without one, or while it does not answer, the footer shows `not running` or `not answering` even if `silent` is stored.

</details>

## References

Coding-agent docs: [Claude status lines](https://code.claude.com/docs/en/statusline), [Claude skills](https://code.claude.com/docs/en/skills), [Claude settings and worktree precedence](https://code.claude.com/docs/en/settings#where-claude-code-keeps-the-local-file-in-a-git-repository), [Codex configuration](https://developers.openai.com/codex/config-reference).
