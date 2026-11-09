# Merge Assist v1: the demo build

**Document type:** As-built specification of the first Merge Assist build, the code on its demo branch.

**Delivery status:** Demo. Experimental and internal testing; off unless a person turns it on for a repository.

**Superseded by:** the [Merge Assist PRD](merge-assist-prd.md), whose target design replaces candidate files with
rules in `.rerail/rules/` and the scan at every session start with an index that updates when a branch moves. Its
"Build status" section lists what of this build carries over. Code comments cite this document's section numbers.

**Prior art:** [Amp comparison](branch-coordination-amp-comparison.md).

## 1. Summary

When several people and their coding agents work on parallel branches, they remake the same decisions and discover
conflicting ones only at merge time. Merge Assist turns each decision a developer makes while steering their agent
("no, do it this way") into a **candidate rule** that travels with the branch. Rerail reads the candidates on other
branches, local and in open pull requests, and acts on each one in one of three tiers:

1. **Absorb.** A compatible decision from a trusted teammate starts steering your agent too, and you can undo it.
2. **Review.** A decision your branch already breaks, or one from someone Rerail cannot vouch for, waits for you to
   adopt or dismiss it.
3. **Talk.** When you and a teammate decided the same question differently, Rerail says so and shows both decisions.
   The two of you resolve it; Rerail does not pick a winner.

Rules are the intermediate representation of architectural intent. A decision that Rerail can turn into a code rule
is checked exactly, by Rerail's own engine. A decision that cannot be a code rule is still shared, and only your own
coding agent judges it.

## 2. Problem

- Learnings are not shared across sessions. A teammate leaves breadcrumb notes; another person's agent cannot use them.
- Discovery is sequential: the same decisions are remade, and the same conflicts are found at merge time.
- Existing tools resolve conflicts after the fact. Amp's **Ship** rebases, tests and resolves textual conflicts when a
  branch lands on `main`, one branch at a time
  ([shipping](https://ampcode.com/docs/orbs/shipping)). Nothing compares two branches that are both still in progress,
  and nothing compares design intent rather than lines.

**Intended value:** move coordination left, before code is written against a decision someone else already made
differently. Re-steering an agent mid-work is cheaper than resolving divergent architecture after both branches land.

## 3. Users and goals

**Primary user:** a developer whose coding agent works on a branch while teammates work on others in the same
repository.

Goals:

1. A decision made once on a branch applies to the rest of that branch's work without the developer repeating it.
2. Decisions on other branches reach your agent before you build against them, when they are compatible.
3. Real disagreements surface as a conversation between people, early, with both decisions side by side.
4. Trivial overlap (adjacent lines, shared files) is not reported as a disagreement.
5. Nothing leaves the machine that the developer did not already send through their own tools.

Non-goals for this version: resolving conflicts automatically, rebasing, posting to pull requests, blocking pushes or
merges, a hosted service, other Git hosts than GitHub for remote pull requests.

## 4. Principles

- **Local first, no Rerail network requests.** Rerail runs tools the developer already has, under their existing
  sign-in: `gh` to read pull requests (read-only), and their coding agent's CLI to judge. Rerail holds no credentials.
- **Git is the sync layer.** Candidates are files on the branch. They move with commits and pushes the developer
  already makes, on whatever Git host the team uses. Rerail never commits or pushes.
- **Deterministic before intelligent.** Exact checks (Rerail's engine, `git merge-tree`) run first; the agent judges
  only what they cannot decide.
- **Innocent until proven guilty, with a visible undo.** Compatible decisions apply automatically and are always shown,
  with one-click undo. Nothing is absorbed silently in the sense of invisibly.
- **Never stronger than steer.** Candidates steer; they never block.
- **People decide disagreements.** Rerail never chooses between two developers' decisions.

## 5. Experience

### 5.1 Turning it on

Merge Assist is off for every repository. A person turns it on in the dashboard (**Merge Assist** page) or with
`rerail merge-assist on`. A coding agent cannot turn it on through Rerail's MCP tools or API (§7). The page states, before the switch:

- Rerail reads corrections you give your coding agent in this repository and asks **your coding agent's CLI** to
  summarise each one as a decision. That sends the correction and the code it corrected to your agent's provider, as
  your session already does.
- Each decision is written to `.rerail/candidates/` in the checkout. Commit it to share it; Rerail never commits.
- Rerail reads other local branches, and open pull requests through **`gh`** with your sign-in, read-only.
- Decisions from teammates with write access may steer your agent; you can undo each one.

It also shows what Rerail found: which agent CLI it will use, whether `gh` is signed in, and whether
`.rerail/candidates/` is ignored by Git (then candidates stay local and the page says so).

### 5.2 Capturing a decision

When a person corrects their agent in a session, Rerail checks the correction at the end of the turn, in the
background, without delaying the agent:

1. A cheap filter (the correction phrases `rerail rules evidence` already recognises) skips ordinary prompts.
2. The agent CLI classifies the correction: a **one-off** fix (dropped), a decision for **this branch**, or a
   **repository** convention, and states it as one sentence with the folders it covers. It reads the decisions in
   force on this branch with the correction and says whether the correction only restates one of them (nothing is
   saved) or **amends** one: adds, removes or changes what that decision asks. An amendment states the whole
   decision as it now stands, may only name a decision it was shown, and keeps covering what that decision covered:
   Rerail adds its scope to the amendment's.
3. When the decision can be a code rule, the agent CLI drafts one: a regex, or an AST check that names a pattern or a
   node kind, optionally narrowed by a node it has or is inside (a `for_statement` that has `request($$$ARGS)`). Its
   test cases are complete files taken from the correction itself: the code the agent wrote is a caught case, the
   corrected code a passed case. Rerail compiles the rule and runs the cases and a repository pass over the checkout,
   which must match nothing: the corrected code and the rest of the branch are clean. A draft that fails gets one
   repair round, in which the agent CLI sees the failure (the compile error, the failing case, or up to three matched
   lines) and corrects the check; the repair counts against the judgment budget like any call. A draft that still
   fails falls back to a **note**: the decision text alone, with the reason recorded.
4. Rerail writes the candidate file and tells the agent, which tells the person in one line:

```text
Rerail saved your correction as a decision for this branch: "Retries live in the transport layer; callers don't wrap
requests in retry loops." It applies from now on. Undo: Merge Assist page, or `rerail merge-assist reject <id>`.
```

A code-rule candidate steers like any rule. A note reaches the agent as guidance once, when it is saved. An amendment
is a new candidate, with its own id, whose `supersedes` names the one it amends; from then on the amendment is in
force on this branch and the decision it amends is not (its file stays, so the change shows in the diff). The page
and `rerail merge-assist status` show "amends …" under an amendment and mark the decision it replaced as amended;
rejecting the amendment puts that decision back in force.

### 5.3 Scanning other branches

A scan runs when a session starts in a checkout with Merge Assist on, after a decision is saved, and on demand (the
page's **Scan now**, `rerail merge-assist scan`, or the `merge_assist_scan` MCP tool). It reads:

- every local branch with commits not on the base branch, at its committed tip;
- every open pull request through `gh pr list`, and its candidates through the GitHub contents API at the pull
  request's head commit.

A branch stacked on yours, or one yours is stacked on, is skipped: its decisions are already yours.

### 5.4 The three tiers

For each candidate on another branch:

| Tier | When | What happens |
| --- | --- | --- |
| Absorb | The author is trusted, your branch's changes pass its code rule, and your agent's CLI finds no contradiction with your own decisions | It applies to this checkout: a code rule steers your agent; a note is delivered once. Listed under **Added from other branches**, with Undo |
| Review | Your branch's changes already break its code rule, or its author is not trusted (no write access, or `gh` unavailable), or your agent's CLI cannot tell | Listed under **Needs your review** with the evidence. **Adopt** applies it; **Dismiss** hides it until it changes |
| Talk | Your agent's CLI finds it contradicts one of your own decisions | Listed first under **Decided differently**, both decisions side by side, the author and pull request, and a message you can copy to send them. Nothing is applied |

`git merge-tree` conflicts between your branch and theirs are shown as context on a Talk or Review item that covers
the same files. On their own, overlapping or adjacent lines are never a finding.

**Trusted** means the candidate came from a pull request whose author has write access to the repository
(`gh api repos/{owner}/{repo}/collaborators/{login}/permission`), or from a local branch on this machine that tracks
no remote other than `origin` and holds no commits of an open pull request whose author lacks write access (a branch
checked out from an outside contributor's pull request is not trusted). An absorbed decision is checked again on every
scan: one whose author is no longer trusted, or whose rule your branch now breaks, goes back to review.

### 5.5 The dashboard page

**Merge Assist** in the sidebar. For the selected checkout:

- A status strip: on or off, the branch, the agent CLI used for judgment, `gh` state, when the last scan ran, and how
  many judgment calls the last hour used against the budget.
- **Decided differently** (Talk), **Needs your review**, **Added from other branches**, and **Decisions on this branch**
  (your own candidates: decision text, whether Rerail checks it exactly or your agent judges it, the folders, when it
  was saved, and **Not a rule**).
- Each finding names where it came from and who is behind it: a pull request's author as GitHub reports it, or, for a
  local branch, the branch and the author its candidate file declares, labelled "declared by". A candidate's own
  `author` field is never shown for a pull request, since anyone can write any name there.
- An empty state that explains what will appear and how to make the first decision.

### 5.6 CLI and MCP

```sh
rerail merge-assist [status] [--json]      # this checkout: state, decisions, findings
rerail merge-assist on | off               # the person's (the CLI runs as the person; see §7)
rerail merge-assist scan [--json]
rerail merge-assist reject <candidate-id>  # "Not a rule" for one of your own decisions
rerail merge-assist adopt <finding-id>     # Review → applied
rerail merge-assist dismiss <finding-id>
rerail merge-assist undo <finding-id>      # remove an absorbed or adopted decision
```

MCP tools (the same operations the dashboard uses): `merge_assist_get`, `merge_assist_scan`,
`merge_assist_candidate_set`, `merge_assist_finding_set`, and the people-only `merge_assist_settings_set`. An agent may
reject one of its own branch's candidates when the person asks, and dismiss a finding that is still open. It may not
adopt, turn the feature on, undo a decision, or dismiss one that applies, since that has undo's effect. These limits
hold for the MCP tools and the HTTP API; the CLI runs as the person (§7).

## 6. Candidate file format

`.rerail/candidates/<id>.json`, one file per decision, written with mode `0644` because it is meant to be committed:

```json
{
  "format": "rerail.candidate/v1",
  "id": "candidate:retries-in-transport-3f9a1c",
  "decision": "Retries live in the transport layer; callers don't wrap requests in retry loops.",
  "generality": "repository",
  "scope": ["src/api/**"],
  "author": "tbelanger",
  "createdAt": "2026-11-03T14:02:11Z",
  "state": "active",
  "rule": { "format": "rerail.policy/v1", "id": "candidate:retries-in-transport-3f9a1c", "...": "..." },
  "signals": ["retry(", "for (let attempt"],
  "supersedes": null
}
```

- `rule` is present only when the decision is a validated code rule; the rule's `id` equals the candidate's `id`, and
  its mode is always `steer`.
- `signals` are at most 8 literal code fragments of 2 to 40 characters, one line each, whose presence in new code means
  the decision is at stake; they are matched case-sensitively as plain substrings, never as patterns. Capture takes the
  judge's signals and the fragments the correction removed (call heads, member-access chains, tagged-template heads and
  keyword phrases such as `throw new`, read by syntax, leaving out names the edited code declares). It tries each in
  shorter forms cut at syntax boundaries (`console.log(` → `console.log` → `console.`) and keeps the most general form
  that occurs in the code before the correction, not in the code after it, and in at most a fifth of the files in
  scope. A kept form that contains another is dropped; the rest are ranked shortest first. The field is optional; a
  decision without it can have signals derived by the judge from its text.
- `state` is `active` or `rejected`. A rejected file stays so that a teammate who already absorbed it sees the
  withdrawal on their next scan.
- `supersedes` is null, or the id of the decision on the same branch this one amends. Among one branch's candidates,
  the ones in force are the active ones no other active candidate supersedes; a scan reads another branch's the same
  way, so a teammate's amendment replaces the decision it amends. An amendment of a missing or rejected candidate is
  an ordinary decision, and amendments that lead back to themselves amend nothing.
- `follows` is absent, or `{ ref, candidateId }`: the decision on another branch this one took on a conflict (§12.4).
  `decision` keeps the text as it was then; delivery resolves the reference to that decision's latest text, carried
  through its amendments, when the repository can read it.
- The file never holds the correction text, code beyond its signals, paths outside `scope`, session ids or anything
  else from the transcript. The link from a candidate to the session and correction it came from stays in the local
  database.

## 7. Security

Merge Assist reads data written by other people (candidate files, pull request metadata) and passes some of it to an
agent CLI and, when absorbed, into the developer's own agent session. The threat model:

| Threat | Mitigation |
| --- | --- |
| A malicious candidate injects instructions into the developer's agent | Only trusted authors' candidates are absorbed; others wait for review. Decision text is capped at 280 characters, stripped of control characters and Markdown links, and delivered inside Rerail's guidance framing. Candidates never exceed `steer` |
| A malicious candidate injects instructions into the judging agent CLI | The judge runs with **no tools**, no MCP servers, no session persistence, in an empty temporary directory, so it cannot read or change the repository. Untrusted text is passed as fenced data with a per-call random fence and an instruction to treat it as data. Its output must match a JSON schema and is used only as a verdict; it never executes |
| A malicious code rule (expensive regex, broad scope) | Rules are parsed with the same schema and compiled in the same kernel as every Rerail rule, which refuses unsafe regexes. Scope globs must be relative and stay inside the repository. A peer candidate whose rule fails to compile is shown for review as a note, never applied |
| Oversized or numerous candidate files | At most 64 candidates per branch and 16 KiB per file; extra files are reported, not read |
| Path tricks in candidate file names or ids | Ids match `^candidate:[a-z0-9-]{1,64}$`; files are read only from `.rerail/candidates/` at the top of a tree, by Git object, never by following a path from the file |
| Recursive Rerail sessions from the judge | The judge runs in a temporary directory that is not a started repository, so Rerail's own hooks ignore it, and with the agent CLI's extension/plugin discovery off where it has a switch |
| Credential exposure | Rerail passes the environment the agent CLI and `gh` need and never reads or stores tokens. `gh` runs with prompts and the update notifier off |
| An agent turning on Merge Assist or adopting decisions to widen its own latitude | On the MCP tools and the HTTP API, turning on, adopting and undoing are people-only: a coding agent's call is refused. An agent may dismiss only a finding that is still open, because dismissing an applied one undoes it. The CLI runs as the person, so an agent with shell access can run `rerail merge-assist on`, `adopt` or `undo`, as it can any Rerail command. The people-only guarantee covers the MCP and API surface, not the shell |
| Runaway spending of the user's tokens | A per-repository budget (default 40 judgment calls an hour), results cached by content hash, and the page shows the use |

## 8. Privacy

Updates to [privacy.md](privacy.md):

- **What goes over the network**: with Merge Assist on, Rerail runs `gh` to list open pull requests and read the
  `.rerail/candidates/` files at their heads. It is read-only and uses the developer's own `gh` sign-in.
- **Your coding agent's provider**: with Merge Assist on, Rerail runs the developer's coding agent CLI to judge
  corrections and decisions. That run sends the correction, the code it corrected, and candidate decisions to the agent's
  provider, under that provider's terms. Rerail makes no model request itself.
- **What Rerail keeps**: candidate files are written into the checkout to be committed; they hold only the decision,
  its scope, the rule and the author's login.

The principle stays: Rerail makes no network requests with credentials of its own.

## 9. Technical design

### 9.1 Modules

| Layer | Module | Responsibility |
| --- | --- | --- |
| engine | `src/engine/merge-assist/candidate.ts` | The `rerail.candidate/v1` schema, limits, sanitising, ids |
| engine | `src/engine/merge-assist/candidate-files.ts` | Read and write candidates in a working tree (atomic, no symlinks); read them from a Git tree object; whether Git ignores the folder |
| engine | `src/engine/merge-assist/branch-git.ts` | Local branches and tips, base branch and merge base, changed paths, ancestry, `merge-tree` conflicts |
| engine | `src/engine/merge-assist/corrections.ts` | The correction phrases and call summaries, moved from `cli/rule-evidence.ts` |
| engine | `src/engine/code-rule/rule-check.ts` | Compile a code rule and run cases and a repository pass, moved from `cli/rule-draft.ts`; optionally limited to given paths |
| engine | `src/engine/storage/schema.ts` | `repositories.merge_assist`, `merge_assist_items`, `merge_assist_judgments` |
| runtime | `src/runtime/merge-assist/agent-judge.ts` | Find the agent CLI, run it headless with a JSON schema, budget and cache |
| runtime | `src/runtime/merge-assist/judge-prompts.ts` | The capture and comparison prompts and schemas |
| runtime | `src/runtime/merge-assist/github.ts` | `gh` reads: auth state, open pull requests, candidates at a commit, author permission |
| runtime | `src/runtime/merge-assist/capture.ts` | Corrections in a session → candidates |
| runtime | `src/runtime/merge-assist/scan.ts` | Peers → tiers → items |
| runtime | `src/runtime/merge-assist/activation.ts` | Which candidates apply to a checkout, reconciled into library rules and per-checkout settings |
| runtime | `src/runtime/merge-assist/service.ts` | The facade the runtime, API and CLI call |
| api | `src/api/merge-assist-contracts.ts`, `merge-assist-operations.ts` | The operations in §5.6 |
| cli | `src/cli/merge-assist-cli.ts` | `rerail merge-assist` |
| ui | `ui/views/merge.js`, `ui/merge-model.js`, `ui/css/merge.css` | The page |

### 9.2 Applying a candidate to one checkout

Library rules are attached per repository and folder, so they would reach every worktree. A candidate applies to one
checkout: Rerail upserts the candidate's rule as a library rule, attaches it at the repository root with the
repository-level setting **off**, and sets it **on** for the checkout. Checkout settings take precedence over
repository settings (`rule-configuration.ts`, `ownersOf`). Reconciliation runs at session start, after capture and
after a scan, from the candidate files in the working tree plus the absorbed and adopted items for that checkout.

### 9.3 Storage

- `repositories.merge_assist` (boolean, default false): migration `repositories-merge-assist`.
- `merge_assist_items`: one row per candidate known to a checkout, own or from a peer: checkout path, repository key,
  candidate id, content hash, origin (`own`, `peer`), peer ref and commit, peer author, tier, status (`open`,
  `applied`, `dismissed`, `rejected`), the evidence and verdict, and for own candidates the session and event they
  came from.
- `merge_assist_judgments`: judgment results keyed by a hash of the task and its inputs, with the runner and time,
  used as the cache and to count the hourly budget.

### 9.4 Agent CLI invocations

The judge must run with no tools at all, MCP servers included, so that text written by other people cannot make it
act. Each invocation was checked against a real run.

| CLI | Command | Notes |
| --- | --- | --- |
| Claude Code | `claude -p --output-format json --json-schema <schema> --tools "" --disallowedTools "mcp__*" --strict-mcp-config --no-session-persistence --max-turns 2` | Reads `structured_output`. Not `--bare`, which would not use the person's subscription |
| Codex | `codex exec --sandbox read-only --skip-git-repo-check --ephemeral --ignore-user-config --ignore-rules --disable shell_tool --disable unified_exec --disable code_mode_host --disable multi_agent --output-schema <file> -o <file> -` | Prompt on stdin. Without the `--disable` switches the model keeps a read-only shell; without `--ignore-user-config` it loads the person's MCP servers and hooks |

OMP and Pi are not judges yet: in print mode their MCP servers stay connected even with `--no-tools`, and neither has a
switch to turn MCP off for one run. A session in OMP or Pi uses Claude Code or Codex when either is installed.

All run in a fresh `0700` temporary directory, with a 120-second timeout, a 1 MiB output cap, and no shell. The
runner preference is the host of the session that triggered the work, then Claude Code, then Codex.

## 10. Demo

Two developers, two branches of the same repository:

1. Jeremy, on `jbarnes/session-retry`, corrects his agent: "don't wrap API calls in retry loops; retries belong in the
   transport". Rerail saves the decision as a code-rule candidate; he commits it and opens a pull request.
2. Tim, on `tbelanger/export-api`, starts a session. The scan absorbs Jeremy's decision; when Tim's agent writes a retry
   loop in `src/api/`, Rerail steers it with Jeremy's rule.
3. Jeremy also decided "API errors are returned as `Result` values, not thrown". Tim had told his agent the opposite
   earlier on his branch. The page shows **Decided differently**, with both decisions and a message to send Jeremy.
4. A third candidate, from a pull request by an outside contributor, waits under **Needs your review**.

## 11. Open questions

- Promotion: when a branch merges, should its candidates become repository rules automatically, or after review on
  `main`?
- Should reviewers' comments on pull requests be a source of decisions?
- Is a per-repository budget the right unit, or per person per day?
- Do teams want candidates committed in the branch, or kept out of the diff (Git notes, a `refs/rerail/` ref) at the
  cost of `gh` not reading them?

## 12. Conflicts (proof of concept)

Two lanes. Settled decisions steer agents: candidates, activation and the notes above, unchanged. In-flight conflicts go
to the person: the agent gets one line of information, never an instruction to align; the person gets one line in the
session and a card on the Merge Assist page's Conflicts tab. Not in the proof of concept: a judge or panel deciding,
and units delivered to agents. A verdict is undone like code: its entry file is reverted on the branch.

### 12.1 Detection

Detection runs on every scan (session start, after a saved decision, Scan now) and after the agent's edits, in the
background and at most once every 30 seconds per checkout; it never holds a hook and never runs while Merge Assist is
off. Mine is the checkout's working changes, committed and not, against the merge base of HEAD with the base branch
(Rerail's own `.rerail/candidates/` left out). Theirs are the live branches: local branches, remote-tracking branches,
and open pull requests whose head the repository holds, read through `gh` when it is signed in (listed at most every
five minutes). Nothing is merged or written to find them.

- **`overlap`**, one conflict per (their ref, file): two `git diff --unified=0` hunks in the same file that touch the
  same base lines or leave at most 3 unchanged lines between them. Renames are ignored, and the same change on both
  sides is not a conflict. A ref's own change is its diff from where it left its base branch, or from this checkout's
  base when that is nearer, so mainline work it merged in does not count and a ref mainline already holds has none.
  The rule lives in `src/engine/merge-assist/overlap.ts`; the replay benchmark imports it from there.
- **`decisions`**: every Talk finding (a trusted teammate's decision contradicting one of this branch's) is also a
  conflict with both decisions' texts. The finding stays, as before.
- **`verdicts`**: two provisional ledger entries on the same question (§12.6), this branch's and a live ref's, with
  different winners. Each side shows its entry's decision; it surfaces like any conflict.

The dimmer decides how loud each open conflict is, on every detection, from what Git measures
(`src/engine/merge-assist/dimmer.ts`, pure; the signals in `conflict-signals.ts`; the pass, budgets and snooze in
`src/runtime/merge-assist/conflict-level.ts`). It interrupts only when continuing costs more than stopping:

- **Signals.** `lockIn` ×4 for a persisted schema, migration or file format (`migrations/`, `*.sql`, `schema.*`,
  `*.proto`, OpenAPI files), a public API (an exported signature changed in a package entry or index file) or a
  retirement (a deleted file or exported symbol); ×2 for another exported signature; ×1 otherwise. `blastRadius`: the
  files in the base that name the contested symbols as a whole word (`git grep -lw`, at most 200), plus the other live
  refs that change the same files. `growthPerHour`: every line both sides added to the contested files since their work
  forked (from `git log --numstat` on theirs and on this branch, plus this checkout's working files), over the hours
  since the fork (at least 1): a stable rate, so the same lines score the same on every pass and a conflict nobody
  builds on only grows quieter. (Not "since detection", as first specified: that window collapsed after the first pass
  and dropped queued conflicts.) Exported names for lock-in come from both sides' full hunks
  at the contested lines, never the 40-line display. `pLose`: 0.5 (no panel runs at detection, and a precedent settles
  a conflict before it is dimmed). `hoursToMerge`: 24 with a pull request open on theirs, else their branch's age, at
  least 24 and at most 336.
- **Score**, in expected rework lines: `pLose × growthPerHour × hoursToMerge × lockIn × (1 + log2(1 + blastRadius))`.
- **Levels**, against `interruptCost` (300 lines by default): `logged` for quiet files (every file a lockfile, a
  changelog, generated output or under `docs/`) or a score under a quarter of it; `ambient` under it; `checkpoint` at or
  above it; `stop` only with lock-in ×4, growth over `stopGrowthPerHour` (50), no verdict or precedent on the question
  in the ledger, and a score of at least four interruptions.
- **Hysteresis.** An open conflict's level never drops, and rises only when its score clears the next threshold by
  20%. A conflict seen for the first time takes its level at once. Resolving it resets the dimmer; one the ledger
  reopens comes back at ambient and rises from there.
- **Budgets.** At most one stop per session (the session whose boundary ran the detection; a detection from the
  dashboard or the CLI grants none) and `checkpointsPerDay` (3) prompts per person per UTC day (`toISOString` date), a
  stop counting as one, kept in `merge_assist_state` under `budget:<person>`. A conflict over budget is queued for the
  level it calls for (`queued: true`, `queuedFor`) and keeps waiting for it, whatever its score does meanwhile, until
  the budget frees (a new session's detection for a stop, the next day for a checkpoint), then rises to it; waiting
  for a stop it shows as a checkpoint while one fits today, else at ambient.
- **Reasons.** One plain sentence per signal (`persisted schema or file format: ×4`, `+380 lines on theirs in 14 h`,
  `14 callers`, `pull request open: about 24 h to merge`, `no verdict yet: even odds either side loses`), shown on the
  card, the agent's checkpoint card and the CLI.

`.rerail/policy.yaml` at the base sets the three numbers under `dimmer:` (`interruptCost`, `stopGrowthPerHour`,
`checkpointsPerDay`). Each conflict in the status carries `level`, `signals`, `score`, `reasons`, `queued`, `snooze`
and a `shortId` (six hex characters the person names it by in chat).

A conflict's id is `conflict:<question>:<8 hex of their ref>` (a `verdicts` conflict's question is prefixed
`verdicts:`, so it never collides with the overlap on the same question and ref). The question is the key ledger
entries share: for an overlap, `questionOf` over the file and the base lines their overlapping change covers, so the
same overlap keeps its id across scans and another overlap in the same file is another question; for a contradiction,
both decisions where their amendment chains start, sorted, with their scopes, so both branches compute the same
question and amending a decision (Keep mine amends this branch's) does not reopen it. An overlap's title is the
symbol enclosing the contested lines (a declaration the hunk itself changes, else the one Git's hunk header names), cut
before its parameters or body and at a word boundary within 60 characters. Each side's summary is the commit that
changed its contested lines: `git blame --porcelain` over just those lines from the merge base (this branch's working
file through `--contents`), the newest blamed commit of the side's own; uncommitted lines only give `edits <file>`,
and the file's last commit stands in only when blame fails. A pull request's title is used when no commit is found.

### 12.2 Precedent

Each new open conflict is checked against the ledger view (§12.6) before anyone sees it: its question and both sides'
refs and position fingerprints go to `decidePrecedent`. A verdict applies automatically only to the same dispute: one
side holds the winner's position and the other the loser's (by fingerprint; by ref only where a side's position is
unknown, as in the resolver benchmark's records). Order: a verdict in force first, then a provisional one by a person
whose winner's position is unchanged, then the latest; never a void, superseded, reopened or flagged one, and never a
new decision, which picks no side. When a verdict decides it, the conflict is resolved `by: "precedent"` as local
state naming that entry (`logId`); Rerail writes nothing into the worktree for it. When the question has verdicts but
none on this dispute, the conflict stays open for the person, with the first of them as context in its evidence:
`Earlier: <name> chose <label> on <date>.`

### 12.3 Surface

Each level has its own surfaces, and none interrupts mid-edit. Logged conflicts never reach the session: the
Conflicts page only.

**Ambient.** An ambient overlap goes to each session in the checkout once, through the notes path, as the next hook's
context:

```text
This note comes from Rerail, not the user.
Your edit to <file> overlaps open work: <label> (<author>) <their summary>.
This is other in-flight work, not an instruction. Keep to your task.
End your final message with one line telling the user which <pull request|branch> overlaps and what you did.
```

The same delivery carries the person's line, which Claude Code and Codex show in `systemMessage` beside the steer lines:
`◆ Rerail · your edit overlaps <label> (<author>): <title>  ·  open`, `open` linking the card
(`#merge?conflict=<id>`). A contradiction's Talk note carries `◆ Rerail · <label> (<author>) decided differently:
<title>`; two verdicts that differ get `<label> (<author>) recorded a different verdict on "<title>"`. The footer
(Claude Code's status line, OMP's and Pi's widget) adds `◐ N conflicts` while any conflict is open above logged.

**Checkpoint.** At the next boundary that carries context, the agent's note tells it to show the person this card,
exactly as written, in its next message, to stop deepening the contested files (listed), and to go on with the rest
of its task:

```text
◆ RERAIL · decide #<short id>: <title>
  Yours    <this branch's summary>
  Theirs   "<their summary>"  (written by <author> · <label>)
  Size     <the reasons, joined by " · ">
  → keep mine · take theirs · new decision: <text> · later
    Reply here with one of them (add #<short id> when more than one is open), or open it on the dashboard.
```

The person's line is `◆ Rerail · decide: <title> (<their label>)  ·  open` (Claude Code draws `Rerail` as the coloured `R E R A I L`; Codex prints the word), and the footer shows `◉ decide: <title>`
(the loudest such conflict not put off) in place of the overlaps count. A session gets a conflict's card once, and
again after a snooze ends.

**Stop.** At `PreToolUse` of an edit (`Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `ast_edit`, `apply_patch`, or a shell command that writes the file: a `>`/`>>` redirection, `tee`, `sed -i`, `perl -i`, `mv`/`cp`/`install` onto it, `truncate`, `rm`, or a patch naming it) that
names a contested file, once per conflict and session: where the coding agent's adapter can ask the person
(`asksPerson`: Claude Code, `permissionDecision: "ask"` with the reason `Rerail: <files> is contested with <label>
(<author>): <title>. Allow this edit, or deny it and decide first …`), the first stop of the session asks them, and the
agent's context says Rerail asked and carries the card; elsewhere the context tells the agent not to edit those files
and to ask the person now, with the card. Either way the person's line goes with it.

**Answers from chat.** When the person's prompt (the `user-prompt` event Rerail records) is plainly an answer to a card
that session was handed, Rerail records it as that person, by the same path as the dashboard (`parseChatAnswer` in
`src/engine/merge-assist/chat-answer.ts`). The answer is the whole message, optionally led by the conflict's name
(`#<short id>` or its title): `keep mine`, `take theirs` or `later` (these three may instead be followed by the name: `take
theirs #<short id>`, `keep mine (#<short id>)`), trailing punctuation allowed, or a message that
starts with `new decision: <text>` (1–600 characters, never the card's `<text>` placeholder). Unnamed, it answers the
one such conflict not put off. Matching is case-insensitive phrases, never a model: prose that mentions an option
("take theirs into account", "I made a new decision: …") records nothing, and so does a message when several are open
and it names none. A prompt in another session (a headless `claude -p` an agent runs from its shell, say) never
answers. `later` snoozes the conflict: no card, no stop and no `◉ decide` until its level rises, a pull request opens
on theirs, or the base branch moves (a merge). A coding agent still cannot resolve one (`merge_assist_conflict_set`
refuses it).

### 12.4 Resolve

Only a person resolves a conflict: the dashboard, `rerail merge-assist resolve`, or `PUT /api/merge-assist/conflicts`.
A coding agent reads conflicts through `merge_assist_get` and gets `409 invalid-state` from `merge_assist_conflict_set`.

| Action | Ledger entry | Decision written on this branch |
| --- | --- | --- |
| `mine` (Keep mine) | verdict `a`; result `decision` | text defaults to this side's summary; on an overlap with its location, `<last commit subject touching the file> (<file>:<line>)`, or `The change on <branch> (<file>:<line>)` while uncommitted |
| `theirs` (Take theirs) | verdict `b`; result `decision` | text defaults to their summary, on an overlap as for `mine`; follows their decision by reference |
| `new` (New decision) | verdict `new`; result `amendment` | the person's text (1–600 characters) |
| `dismiss` (Not a conflict) | none | none; it stays dismissed for that question and their ref |

The decision is a candidate in `.rerail/candidates/`, active, authored by the person (Git's `user.name`, else the
account name), scoped to the conflict's files; for a contradiction or a verdicts conflict it amends this branch's
decision. Taking theirs on a contradiction or a verdicts conflict gives the candidate `follows: { ref, candidateId }`,
naming their decision, with its text as of now. The entry goes to `.rerail/decisions/<id>.json` with the candidate's
path as its `rule`, and replaces (`supersedes`) this branch's latest entry on the question. Both files are written into
the working tree; Rerail commits nothing. From then on lane 1 carries the decision: every session in the checkout gets
a note of it once, a running one at its very next hook (the runtime offers pending notes at every boundary, so a
verdict given on the dashboard or in another process reaches it), and teammates get it once the person commits it. A
decision that follows another is delivered with the followed decision's latest text, which each detection resolves
(`followedDecision`), and is news again when that text changes. The status's `resolution` carries the entry id and the
rule path.

### 12.5 Storage, status, API and CLI

Conflicts live in `merge_assist_state` under `conflicts:<checkout path>`, open, resolved and dismissed, each with its
files, its record and both sides' positions; turning Merge Assist off for the repository clears them, dismissals
included. The status gains `conflicts` (open ones loudest first, then the latest 20 resolved, then the latest 20
dismissed) and
`decisionLog`: the ledger view grouped by question (`questions`, the question with the newest entry first, at most 50
entries), each item `{ id, question, title, status, verdict, outcome, by, source: { ref, label }, recordedAt, rule,
text, follows }`, with the entries Rerail could not read as `problems`. The dashboard's Decision log tab shows each
question with a status chip per entry, the branch it was read from, and a filter by status.

```sh
rerail merge-assist conflicts [--repo <dir>] [--json]                              # every conflict, with both hunks, and the decision ledger by question
rerail merge-assist resolve <conflict-id> mine|theirs|new|dismiss [--text "..."]   # the person's verdict
```

`rerail merge-assist status` gains a Conflicts section; each conflict line shows its level (with `queued` or `later`
when they apply) and the dimmer's reasons on a `Size:` line. Modules: `src/engine/merge-assist/{overlap,conflicts}.ts`
(the rule, ids, the dimmer), `src/engine/merge-assist/{decision-entry,decision-files,ledger,commit-staging}.ts` (the
ledger) and `src/runtime/merge-assist/{conflict-detect,conflict-resolve,conflict-store,ledger-store}.ts`.

### 12.6 The decision ledger

Entries travel with the branch whose person decided them; the ledger is a view computed from Git, never written.

**Entry files.** `.rerail/decisions/<id>.json`, format `rerail.decision/v2`, mode `0644`. The id is the first 32 hex
digits of sha256 over the canonical JSON (keys sorted) of the entry without `id`, so an edited entry no longer
matches its name and is refused. Fields: `question`; `title`; `base` (the commit both positions are taken against);
`sides.a` (the recording branch) and `sides.b`, each `{ ref, label, sha, author, fingerprint, at }`; `verdict` (`a`,
`b` or `new`); `decidedBy` (`{ kind: "person", name }` in the product; the resolver benchmark also records
`{ kind: "precedent", entry }` and `{ kind: "policy", step }`); `result` `{ candidateId, kind: "decision" | "amendment", text, rule,
follows }`; `supersedes` (an entry id or null); `recordedAt`. A side's `fingerprint` is sha256 of its position, and `at`
says what it is taken over: for an overlap its normalised hunk lines at the contested base lines (`-`/`+` lines,
whitespace collapsed); for a contradiction its candidate's content hash. Escalations are not entries: an unanswered
conflict stays open in local state.

**The view** (`readLedger`) reads the base branch's tree, this branch (HEAD's tree and the working files) and every live
ref (local and remote-tracking branches but this one and the base, and open pull request heads the repository holds),
none the base already holds, deduplicated by id. Status comes from where an entry sits:

| Status | Where |
| --- | --- |
| `in-force` | in the base tree, and the merge held the winner's position: checked once, at the first base commit (along first parents) holding the entry file, so later base work never changes it (side a carries the entry, so its position is expected there; side b only if its commit was in the base by then; a new decision's is its rule file) |
| `superseded` | in the base tree, and a later base entry on the question names it in `supersedes` or is newer (a precedent copy never replaces the human verdict it followed); or, off the base, another entry from the same branch supersedes it |
| `flagged` | in the base tree, but the merge did not hold the winner's position: it contradicted the entry |
| `provisional` | only on this branch or a live ref |
| `reopened` | provisional, and the winner's position at its committed tip (never a checkout's working files) has another fingerprint; never for the checkout whose branch holds the winning position, since reopening tells the side that followed |
| `void` | seen before by this checkout, now on no live ref and not in the base: its branch merged without it, or went away |

**The merge check** runs at every scan and detection pass: the view is read with the checkout's memory of what it saw
(`ledger:<checkout path>` in `merge_assist_state`, every entry found with where, the newest 200 gone ones kept), so an
entry whose ref merged into the base ends `in-force`, `flagged` or `void`. A settled conflict whose entry is now
`reopened` is open again when the pass finds it again, quietly: at `ambient`, the dimmer's state reset.

**Cost.** Live refs come from one `git for-each-ref --no-merged=<base>`, each tip's `.rerail/decisions` tree from one
`git cat-file --batch-check`; only a folder tree the memory has not read is listed and read (trees never change), a
merge is checked once per entry, and a winner tip once per entry and tip. The memory also keeps the last view, which
status reads (the dashboard's polls, `merge_assist_get`, the CLI) show without running Git; a verdict refreshes it at
once. On this repository (426 refs, no entries) a read went from 11 s to under 0.1 s.

**Delivery of verdicts not in force.** Another branch's verdict that has not merged (its entry provisional or reopened)
reaches this checkout's agents only as information, never as a rule: when the scan applies its decision, the agent
reads `<branch> recorded a verdict on a conflict that has not merged yet.`, their decision labelled as theirs, then
`It is provisional: information about work in flight, not a decision to follow.` While a `verdicts` conflict on its
question is open here, neither side's decision is delivered at all: the conflict itself is the signal. Activation never
makes such a decision a rule, whatever it carries. The status says so: its finding has `held: "provisional"` (or
`"contested"` while the verdicts conflict is open), level `note`, and a reason saying it is not applied; the dashboard
and the CLI show "Provisional verdict" or "Held: verdicts conflict open" instead of "Added automatically".

A conflict side's `tip` is `{ at, pushed }`: `pushed` only when its commit is a pull request's head or on a
remote-tracking branch, so a local branch never pushed reads "committed <time>"; this branch's side has none.

**Commit staging.** At the agent's `git commit` tool call (PreToolUse on a shell tool), when files under
`.rerail/decisions/` or `.rerail/candidates/` are untracked, or modified and not staged (with `-a`/`--all`, only
untracked ones), the call's answer carries `Rerail: include these decision files in this commit: <list>`, once per
commit attempt. It is context, never a block, and Rerail stages nothing.
