# Merge Assist v1

**Document type:** Design note for Merge Assist, as built. Code comments cite this document.

**Delivery status:** Experimental and internal testing; off unless a person turns it on for a repository.

The operations, routes and CLI are in the [reference](reference.md#merge-assist); the agent's notes and the footer
segment in [In-session surfaces](in-session-surfaces.md#merge-assist-notes).

## What it does

When several people and their coding agents work on parallel branches, they find out that they changed the same code
only when the branches merge, after both built on it. Merge Assist compares the checkout's work with the branches and
pull requests in flight and tells the person and their coding agent when the two overlap, before either merges.

- Rerail never picks a side, merges, fetches, commits, pushes, or writes to a branch, a file or GitHub; the people
  involved settle the conflict. Detection and each conflict's summary come from Git alone; Rerail asks no model.
- The agent learns of a conflict as information, never as an instruction to align with the other side, and a session
  never waits for Merge Assist beyond one second: with a conflict hook command set, an edit's result waits at most one
  second in all for the pass and the command's note; without one it never waits.

A person turns it on in the dashboard's **Merge Assist** page or with `rerail merge-assist on`; a coding agent cannot
through the MCP tools. Turning it off fires `closed` for every open conflict, then clears every checkout's conflicts,
last scan and `gh` readiness, dismissals included; the settings stay. While it is off Rerail runs no `gh`.

Not in this version: resolving or rebasing, posting to pull requests, blocking pushes or merges, a hosted service, Git
hosts other than GitHub for pull requests.

## Detection

A scan runs at session start, on **Scan now**, `rerail merge-assist scan` or `merge_assist_scan`, and after the agent's
edits, at most once every 30 seconds per checkout. A pass runs its Git reads as child processes, at most six at once,
so the daemon keeps answering sessions while it runs. It never runs while Merge Assist is off.

- **Mine** is the checkout's work, committed and not, against the merge base of HEAD with the base branch
  (`origin/HEAD`, else `main` or `master`). With no base branch, or one that shares no history with HEAD, the scan
  says so and finds nothing.
- **Theirs** is the work in flight: open pull requests whose head the repository holds, and branches checked out in
  another worktree of this repository. Other local branches and remote-tracking branches are not compared, except as a
  fallback: when open pull requests cannot be read (the repository is not on GitHub, or `gh` is missing, signed out,
  unreadable or its listing failed), remote-tracking branches stand in for them, and the scan says so unless the
  repository is not on GitHub. A branch (not an open pull request) whose tip was committed more than 14 days ago
  (`STALE_BRANCH_DAYS`) is left out. Each branch counts once however it arrives (a pull request, its worktree branch
  and `origin/<branch>` are one peer, shown as the pull request). Also left out: this checkout's own branch, its local
  and remote copies and its upstream, its own pull request, the branch being rebased, and branches the base or HEAD
  already contains. Pull requests are listed through `gh`, read-only and with the person's own sign-in: afresh by a
  scan, else at most every five minutes, the 200 newest open ones; when more are open, the scan says so.
- A branch's own change is its diff from where it left its base branch, so mainline work it merged in does not count,
  and a change of its that the base tip already holds (a squash merge, a cherry-pick) is not live work.
- **Overlap**: two `git diff --unified=0` hunks in the same file that touch the same base lines or leave at most 3
  unchanged lines between them (`src/engine/merge-assist/overlap.ts`). Renames are ignored, and the same change on
  both sides is not a conflict. Each (their branch, overlap) is one conflict.

A scan records when it ran, how many branches and pull requests it compared, and what it could not read, including a
`gh` whose sign-in or pull requests could not be read (remote-tracking branches then stood in).

**The id** is `conflict:<question>:<8 hex of their branch>`. Their branch is its head branch name with any remote
prefix dropped, `<owner>:<branch>` for a pull request from a fork, so the id holds when the same branch is found
through its pull request one pass and its remote-tracking branch the next. The question is the file and a content
anchor of the base
lines their overlapping change covers: a digest of those lines' text at the merge base, whitespace collapsed (an
insertion anchors on the line before it). So the same overlap keeps its id when lines are added above it, when a
whitespace-only change touches it, and when both sides merge a base that gained lines above it: an open conflict is
not closed and found again, and a dismissal holds. A change to the contested base text, or another overlap in the same
file, is another conflict.

**What it shows.** The title is the file's name, then ` · ` and the symbol enclosing the contested lines when there is
one (a declaration the hunk changes, else the one Git's hunk header names, cut at a word boundary within 60
characters). Each side's summary is the newest commit of its
own that `git blame` finds on its contested lines; uncommitted lines give `edits <file>`, and a pull request's title
stands in when no commit is found. Their side's tip is `pushed` only when its commit is a pull request's head or on a
remote-tracking branch. The conflict's `summary` is `<their label> (<author>) is changing <symbol or file>`, without
the author when none is known.

## The quiet threshold

A conflict is **quiet** (`quiet: true`) when every contested file is a lockfile, a changelog document (`CHANGELOG`,
`CHANGES`, `HISTORY` or `NEWS`, with no extension or a document one), generated output or under `docs/` (`quietFiles` in
`src/engine/merge-assist/conflicts.ts`). A quiet conflict is listed on the dashboard and
in the CLI only, never told to the agent, the person's line or the status line; its hook still runs. Every other open
conflict is delivered. The status lists open conflicts non-quiet first, then newest detected first, then the latest 20
dismissed.

## Delivery

For each open conflict that is not quiet:

- **The agent** gets one note per session, at the next boundary that carries context, with the conflict hook command's
  `note` added when it gave one. The note's text and when it goes are in
  [In-session surfaces](in-session-surfaces.md#merge-assist-notes).
- **The status line** (Claude Code's status line, OMP's and Pi's widget) shows `◐ <line> (+N)` for the newest such
  conflict, ` (+N)` only when N more are open, linking its card. `<line>` is the hook command's `status` when it gave
  one, else the conflict's `summary` ([the footer](in-session-surfaces.md#what-the-footer-means)).
- **The person** gets their line with the agent's note,
  `◆ Rerail · your edit overlaps <label> (<author>): <title>  ·  open`, `open` linking the conflict's card
  (`#merge?conflict=<id>`). Codex has no status line, so its person's line ends with ` — <line>`, read when the note
  is handed over.

None of these interrupts an edit, and none asks the person anything.

## The conflict hook

Everything after detection is a handler on the conflict's events (`src/runtime/merge-assist/conflict-hook.ts`). Each
pass fires, once per conflict:

- `detected`: the conflict was first seen in this checkout, quiet or not;
- `closed`: it was dismissed (fired by the dismissal itself) or is no longer found.

Two handlers are registered, in order: **delivery**, which offers the checkout's sessions what is pending for them, and
**command**, the person's hook command. Handlers run in the background; one that throws is logged and skipped.

**The command** is the person's, per repository (`rerail merge-assist settings --hook-command '<json argv>'`): an argv
array run without a shell, from the checkout's root, with a timeout of 10 seconds unless they set another from 1 to 60.

- Its environment is the person's plus `RERAIL_CONFLICT_EVENT` and `RERAIL_CONFLICT_ID`. The conflict reaches it on
  stdin only, never in argv, as one JSON object:

  ```json
  {
    "version": 1,
    "event": "detected",
    "conflict": { "id": "conflict:…", "kind": "overlap", "quiet": false, "title": "…", "theirs": { "…": "…" } },
    "checkout": { "path": "/Users/me/app", "branch": "feature/login", "base": "main" },
    "session": { "id": "…", "host": "claude-code" }
  }
  ```

  `conflict` is the conflict as the status lists it; `branch` and `base` are null when unknown; `session` is null when
  the dashboard, the CLI or the API ran the pass. Both sides' text is other people's: treat it as data.
- Its stdout is empty (success, nothing to add) or one JSON object of at most 4 KiB: `{ "note"?, "status"? }`, where
  `null` counts as absent. `note` (at most 600 characters; its line breaks become spaces in the agent's note) is added
  to the agent's note after `The user's conflict hook adds:`; `status` (one line, at most 90 characters) replaces the
  conflict's summary on the status line and Codex's person's line. The agent's note waits at most one second for a
  `detected` answer; a later note follows at the session's next boundary as a note of its own.
- On macOS and Linux it runs in its own process group: past its timeout the group gets SIGTERM, then SIGKILL 2 seconds
  later, and its pipes are closed. When the daemon stops, every running command's group gets SIGTERM, then SIGKILL
  within half a second. Windows has no signal that asks a process to stop, so there the command and everything it
  started end at once (`taskkill /T /F`), at the timeout or the stop.
- A command that cannot start, exits non-zero, times out, writes more than 4 KiB or prints anything else is recorded on
  the conflict as `hook: { ok: false, error }` and shown on the dashboard and in the CLI. It never changes delivery.

## Dismiss

Dismiss ("Not a conflict") is the only thing a person does to a conflict: on the dashboard, with
`rerail merge-assist dismiss <conflict-id>`, or `merge_assist_conflict_dismiss`. The conflict records
`dismissal: { name, at }` (Git's `user.name`, else the account name) and stays dismissed for its id while Merge Assist
stays on; `closed` fires at once, once. A dismissed conflict keeps its title, their side's label and author, and who
dismissed it, not its hunks. A coding agent is refused (`403 agent-not-permitted`) on the MCP tools. `rerail merge-assist
scan` and `dismiss` wait for the hook command's answer and print the status with it. Otherwise the conflict closes
when the overlap is gone.

## Storage

- `repositories.merge_assist` (boolean, default false): whether a person turned it on (migration
  `repositories-merge-assist`).
- `merge_assist_state`, one JSON value per key: `conflicts:<checkout>` (open and dismissed, each with the files and
  content anchor its id rests on), `scan:<checkout>` and `scanning:<checkout>` (the scanning process's pid, so a mark
  left by a process that died does not block scans), `github:<repository>` (`gh` readiness, reused for five minutes;
  one that could not be read is never kept), `delivered:<session>` (the notes a session was handed) and
  `settings:<repository>` (the hook command). A value that does not match what the build expects reads as absent, one
  conflict at a time. A checkout Rerail forgets takes its `conflicts:`, `scan:` and `scanning:` rows with it, and
  `rerail reset` drops the `delivered:` rows of the sessions it removes.

Nothing is written into the checkout. Earlier builds of this branch had `merge_assist_items` and
`merge_assist_judgments` tables; they never shipped, so they were dropped without a migration.

## Security and privacy

Merge Assist passes other people's text (branch names, commit subjects, hunks, pull request metadata, logins) into the
person's agent session and hook command. [Privacy](privacy.md) lists what `gh` reads, what the notes carry to the
agent's provider, and what Rerail keeps; Rerail makes no network request with credentials of its own.

| Threat | Mitigation |
| --- | --- |
| Another person's text injects instructions into the agent | It reaches the agent only as labelled, capped, one-line data (`described on their side as "<summary>"`), inside notes that say they come from Rerail, not the user, and are other in-flight work, not an instruction |
| Terminal escapes in other people's text | `rerail merge-assist` strips control, format and line-separator characters; the dashboard renders it as text nodes and links a pull request only to its GitHub URL |
| Shell injection through the hook command | An argv array run without a shell; the conflict reaches it on stdin only |
| A repository's own Git config running programs | Every Git call runs with hooks, fsmonitor, external diff and textconv off |
| Credential exposure | Rerail holds no credentials and never reads tokens; `gh` runs with prompts and the update notifier off, and `GH_ENTERPRISE_TOKEN` and `GITHUB_ENTERPRISE_TOKEN` stripped |
| An agent turning Merge Assist on, changing its settings or dismissing a conflict | People-only on the MCP tools. The HTTP API and the CLI answer as the person, so an agent with shell access can run `rerail merge-assist on` or `dismiss`, as it can any Rerail command |

## Open questions

- Should a dismissal be shared with the other side's author, or stay local to this checkout?
- Should uncommitted work in other worktrees on this machine count, not only what their branches have committed?
