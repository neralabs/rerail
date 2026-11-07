# Rerail in-session surfaces

What Rerail shows inside a coding-agent session: a note to the agent when the session starts, a steer when a rule is broken, a footer (status line) with the session's counts, and `/rerail` on request.

| Coding agent | Footer | On request | Session identity |
| --- | --- | --- | --- |
| Claude Code | Status line, beside your existing one | `/rerail` | Claude Code's session id and working directory |
| OMP | One widget below the editor; the native footer stays | `/rerail` | OMP's session id and working directory |
| Pi | One widget below the editor; the native footer stays | `/rerail` | Pi's session id and working directory |
| Codex | None: Codex has no slot for one | `$rerail` | Codex's own thread id, never the latest session in a directory |
| Cursor | None | `/rerail` | Cursor cannot name its session, so `/rerail` shows the repository-level form |

## Setup

```sh
npx @neralabs/rerail setup
```

Setup registers every surface below; restart your coding agent sessions afterward. Nothing else needs turning on. Rerail does not install the coding agents themselves.

Every coding agent gets one command set: `/rerail` (what Rerail is doing here, and every command), `/rerail-manage` (open the dashboard), `/rerail-create` (create a rule in your own words), `/rerail-stop`, `/rerail-start`, `/rerail-update`, `/rerail-uninstall` (remove Rerail from this computer; rules and history kept in a backup), `/rerail-help` (the same as `/rerail`), `/rerail-show-me` (a demo: break a throwaway rule, get steered, fix it, clean up) and `/rerail-bug-report` (fix your own rule, or report a Rerail bug in an issue). Settings, status, update checks and reset are terminal commands only (`rerail settings`, `rerail status`, `rerail update check`, `rerail reset`), as are audits (`rerail scan`, in [internal testing](../README.md#internal-testing)).

- **OMP and Pi.** The extension registers the commands natively. `/rerail-create` and `/rerail-show-me` hand their instructions to the agent; `/rerail-uninstall` asks you with a selector.
- **Claude Code.** Setup registers hooks, adds Rerail to your existing status line, and writes the commands to `~/.claude/commands/` with the tool permissions they need. `/rerail-start` may run set-up's commands without Claude Code asking again, so in a repository that is not set up the agent relays each set-up question to you. `/rerail-uninstall` is not pre-approved: Claude Code asks before it runs. The status line refreshes every five seconds unless you set your own interval.
- **Codex.** Setup writes one skill per command under `~/.codex/skills/<name>/SKILL.md`, invoked as `$rerail`, `$rerail-manage`, and so on. Codex's hook trust and command approval still apply: Codex may ask you before `$rerail` reads the local daemon.
- **Cursor.** Setup writes the same commands to `~/.cursor/commands/`.

Claude Code keeps your status line. Rerail's output is added to whatever command you had, including one a Git worktree inherits from its main checkout; a later edit to that shared command applies without reinstalling. Setup run again changes nothing, and `rerail install --remove` restores your previous status line and refresh interval and leaves your other settings as they are. The record of what Rerail replaced is kept in `~/.rerail/harness.sqlite`.

## Session-start note

In a repository where Rerail is started, each session begins with a short note to the agent that you do not see. It tells the agent:

- Rerail is on and stays silent until a rule is broken, and what a steer looks like.
- A steer is advice: fix it, or tell you in one line why not, and carry on.
- To answer "why?" from the steer: the rule id and what it matched.
- To ask you before turning a rule off or changing its mode, and never to turn a rule off or stop Rerail on its own to get past one.
- The session's dashboard link, and that `/rerail` (Codex: `$rerail`) lists the commands.

In Cursor it also asks the agent to begin its reply with each steer's line, since Cursor shows you nothing from a hook. Rerail never writes instruction files. The agent's replies are its own: the note and the steer guide them, nothing enforces them.

## `/rerail`

`/rerail` (Codex: `$rerail`; terminal: `rerail session [<host> [<session-id>]]`) shows what Rerail is doing in this session, then every command:

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

Where Rerail is not started it says so and points at `/rerail-start`; where no daemon answers it says `✕ Rerail is not running.` with how to start it. In Cursor, which cannot name its session, it shows the repository's rule count and a link to its rules instead.

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

Findings are listed in the order they were found: the place (`path:line:col`, the path alone for a finding about a whole file, nothing when there is no path), the message, then the rule id in brackets. A suggested fix follows its finding: `Suggested fix: delete <range>`, `Suggested fix: replace <range> with <code>`, or, over several lines, `Suggested fix, replacing <range>:` and the fix in a `~~~` fence. Ranges use an en dash (`3:1–5:2`); a fix over 240 characters is cut with `… (truncated)`. The block has none of Rerail's own vocabulary (glyphs, modes, packs, links).

Text quoted from the repository (paths, captured code, fixes) is cleaned before it reaches the agent: control and invisible characters are removed, and line breaks inside a line become spaces. Each part is capped and marked `…` where cut: a place, rule id or tool name at 300 characters, a message line at 600, a whole message at 2,000, and each captured snippet at its first line and 80 characters.

The lead says what Rerail did:

| Trigger | Lead |
| --- | --- |
| An edit's check | `Rerail found 1 problem in your last edit:` (`N problems`) |
| A check run again after its sources changed | `Rerail re-checked; still open:` |
| Findings that reach the agent after it moved on | `Rerail finished checking your earlier edits after you moved on:` |
| A repository audit | `Rerail checked the repository and found 1 problem:` (`N problems`) |
| A tool call or conversation rule | `Rerail has guidance for this <Tool> call:`, or `Rerail has guidance for this step:` when no tool is named |

The latitude closes it: ``Fix it, or tell the user in one line why you won't and run `rerail feedback <rule-id> '<that line>'`.`` (`Fix these, …` for more than one finding; `Follow it` and `Follow these` for a tool call's guidance). The command names the rule when the block has one, and keeps `<rule-id>` for the agent to fill in when it has several; `rerail feedback` keeps the reason on this computer ([Steer feedback](reference.md#steer-feedback)). A Block-mode rule steers the same way, since blocking is not built yet. A Log-mode rule never reaches a session.

### What you see

One line per rule: `◆ Rerail · <rule id>  steered  ·  why?`, where `why?` opens the rule on your dashboard. Where the coding agent draws colour (Claude Code, OMP) Rerail is the footer's coloured `R E R A I L` mark; Codex and Cursor show the word. `why?` appears only while a daemon answers on a loopback address; with none, or one bound to `0.0.0.0`, the line has no link rather than one your browser cannot open.

| Coding agent | Your line | A steer found as the turn ends |
| --- | --- | --- |
| Claude Code | Under the tool call, after Claude's `PostToolUse:<Tool> says:`. `why?` is a clickable link in a terminal known to draw links ([below](#links-in-the-terminal)), else `why? → <url>`. Two or more lines start on their own line. | The line shows when the turn ends; the block reaches the agent at its next tool call or session start, led as a late steer. It never rides your next prompt. |
| Codex | Under the tool call (`↳ Hook · …`), plain text: `… steered  ·  why? → <url>`. | The line shows when the turn ends; the block reaches the agent with the next hook that carries context (your next prompt, a tool call, a session start), led as a late steer. |
| Cursor | None of Rerail's own: the agent is asked to begin its reply with `… steered  ·  [why?](<url>)`. | Waits for the next tool result or session start. |
| OMP | Rerail draws the line in OMP's theme colours, `why?` a link where the terminal draws them. Ctrl+O expands it to show where each rule comes from and the block, dimmed. A steer that rides an edit's tool result gets its line right after that call's results, before the next model call; the agent never sees that line. | The run waits up to 10 seconds for the turn's own check, then any pending steers continue the run before it reports done. A steer that arrives after the session is idle waits for your next message. |
| Pi | Pi's default message card, which shows the whole block: Pi's extension API gives Rerail nothing to draw its own line with. | As OMP. |

Glyphs: `◆ steered`, `○ logged`. A Log-mode rule never reaches a session, so sessions show `◆` only.

### When a steer arrives

After an edit, Claude Code, Codex and Cursor wait up to one second for that edit's check before answering, so the steer lands under the edit and the agent can act on it in the same turn; a check usually takes tens of milliseconds. OMP and Pi wait up to 10 seconds at a tool result and at a turn end, so a shell command that wrote thousands of files does not stall the agent for minutes. A steer whose check takes longer goes out at the next hook, reply or poll. These waits are fixed, not set per rule.

A steer never becomes your message, before or after the agent compacts its context. In OMP and Pi a steer from an edit's own check goes into that edit's tool result; a later one is a Rerail message, which the compaction summary labels as an automated rule check, not a message from you. Claude Code and Cursor carry steers as context beside a tool result or at session start, never with your prompt. Codex carries them as a developer message, which can arrive with your next prompt but is not part of it.

[Experimental steer features](internal-testing.md#experimental-steer-features) (the missed-steer digest, and the `rewind` and `subagent` strategies) are off unless you turn them on.

### Edits made by shell commands and before rules are ready

Rerail checks files a shell command wrote once the command finishes. A shell command's result, or a turn end, waits up to 10 seconds for Rerail to find what the shell wrote; what is found later is checked and steered at a later step. The session's dashboard timeline then says rules had not yet seen shell edits at that step. A file the shell deleted is reported only if Rerail had seen it before.

When a session starts in a large repository, Rerail indexes it before code rules can run, and nothing waits for that. Edits made meanwhile are not checked at their own result: the first pass after indexing checks every file that changed. OMP's footer shows ` · indexing` while it lasts (only when it takes over half a second), and the dashboard timeline says code rules were not yet running at those steps.

### When the daemon is down

A hook never fails a tool call. Only a session start may start the daemon; any other hook that finds none running lets the call go at once, so a stopped daemon is not a `hook error` under every call. In OMP and Pi a Rerail tool hook lets the tool go after 25 seconds whatever happens. The footer says `not running`; Codex, which has no footer, shows `Rerail ✕ not running · start it with: rerail daemon start` once, when a session starts.

Events the OMP or Pi extension cannot hand to the daemon are recorded as a gap, not lost silently. The footer adds `Rerail: N session events dropped — daemon not answering` while it lasts (not under `silent`), and the session's dashboard timeline says how many events never reached Rerail.

## What the footer means

```text
R E R A I L ● 26 rules · steered 3×
```

` · steered N×` appears only above zero. Other states:

| Footer | Meaning |
| --- | --- |
| `R E R A I L ○ not on here` | Rerail is not started here, or was stopped |
| `R E R A I L ○ not set up · rerail start` | Started, but the repository has no rules set up yet |
| `R E R A I L ✕ not running` | No daemon answers |
| `R E R A I L ● session unavailable` | The session is not recorded yet |

Appended when they apply:

- ` · worktree`, after the rule count, while the session's checkout runs rules of its own from its `.rerail/rules` folder ([a worktree's own rules](rule-packs.md#a-worktrees-own-rules)); it links to their pack page: `R E R A I L ● 27 rules · worktree · steered 3×`.
- ` · N to approve` while N rules or packs on in this checkout run in Log waiting for your approval ([approval](rule-scoping.md#approval)); a pack counts once, and it links to the repository's Overview, which lists them.
- ` · indexing` in OMP while the repository is indexed ([above](#edits-made-by-shell-commands-and-before-rules-are-ready)). Pi does not show it.
- `engine unavailable` when the rule engine could not load.
- `audit failed (N failures)` when the latest audit failed; N counts every failure, including those its list of at most 20 leaves out.
- For a completed audit with gaps instead: `N files not checked`, `N rules ran out of work` and `N rules stopped by file work`.

The footer refreshes every five seconds and never animates.

What the numbers mean:

- **Rules** is the coverage configured for this session: the code and conversation rules that are on, including any changes made during the session. It is not the rules matching the current file, nor the checks that passed.
- **Steered** counts one per line you saw: two rules in one edit count 2, and a delivery that names no rule counts 1. A Claude Code steer found as the turn ends counts once, when its line shows. Logs, matches, queued guidance and failed sends do not count.
- **Unknown is not zero.** Older sessions, and history lost in a reset, show `steering unavailable`; while coverage is being recomputed the footer shows `rules unavailable`.
- A steer proves neither that the agent read it nor that it fixed anything. A crash right after a steer is sent can leave it uncounted.
- Violations are not a to-do list or a sign that work is ready for review. Rerail running normally and an audit reporting failures are separate states.

`R E R A I L` and the steer count link to this session on the dashboard; the rule count links to the rules page. The six letters each have their own colour, from `#A5ADFB` to `#ECDAE4`, in 24-bit where `COLORTERM` is `truecolor` or `24bit`, else the nearest of 256 colours, and none under `NO_COLOR`. The state mark takes its state's colour (`●` green, `○` dim, `✕` red); nothing else is coloured.

The footer only reads: refreshing it never starts a daemon, runs a check or writes anything. Full audits and rule lists are on the dashboard.

### Links in the terminal

Rerail's steer lines in Claude Code and OMP are clickable (OSC 8) links in terminals known to draw them: kitty, Ghostty, WezTerm, iTerm2, VS Code, Alacritty, Windows Terminal and Orca, and inside tmux 3.4 or later; never inside GNU screen. Elsewhere the URL is printed. `RERAIL_NO_HYPERLINKS=1` or `RERAIL_FORCE_HYPERLINKS=1` (or OMP's `PI_NO_HYPERLINKS` / `PI_FORCE_HYPERLINKS`) overrides the choice. The footer always carries its links; whether you can click them depends on your terminal.

## Session links

```text
http://127.0.0.1:<daemon-port>/?sessionId=<encoded-id>&host=<encoded-host>#sessions
```

A session link opens the dashboard's Sessions view on that session, and keeps it selected across reloads and Back/Forward. A link to a session that is not in the repository the dashboard shows says so in a notice and shows the most recent session instead. A session marked live has no recorded end; it does not prove the coding agent is still running.

## Hiding the footer

Presentation is separate from enforcement: `silent` hides the footer (and the dropped-events line) but does not turn rules off, hold back steers or change modes. `/rerail`, `$rerail` and the CLI and API still answer.

Each preference is `compact` (the default) or `silent`, keyed by repository and coding agent; an empty repository or agent means all of them. The most specific match wins: repository and agent, then repository, then agent, then the database default, then `compact`. Removing an override reveals the next one. Preferences live in the Rerail database and survive `rerail reset`; there is no settings screen or file for them yet. Set them through the daemon's API (also an MCP tool):

| Operation | Purpose |
| --- | --- |
| `GET /api/presentation-preferences` | List stored preferences |
| `PUT /api/presentation-preferences` | Set `{ repository, host, presence }` |
| `DELETE /api/presentation-preferences?repository=…&host=…` | Remove an override and reveal the next default |
| `GET /api/presentation?repository=…&host=…[&sessionId=…&onDemand=true]` | Resolve the preference; optionally include the session's counts |

`repository` is an absolute path; symlinked spellings of one path (`/tmp`, `/private/tmp`) are the same repository. A change shows at the footer's next refresh. Preferences are read through the daemon, so with none running the footer shows `not running` even where `silent` is stored.

## References

[Claude status lines](https://code.claude.com/docs/en/statusline), [Claude skills](https://code.claude.com/docs/en/skills), [Claude settings and worktree precedence](https://code.claude.com/docs/en/settings#where-claude-code-keeps-the-local-file-in-a-git-repository), [Codex configuration](https://developers.openai.com/codex/config-reference).
