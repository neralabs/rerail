# PRD: Merge Assist — shared decisions between branches, before they merge

**Document type:** Product requirements and target design for an experimental feature.

**Delivery status:** Design. A first version runs on the demo branch, specified in
[merge-assist-v1-build.md](merge-assist-v1-build.md); [§14](#14-build-status) lists what it builds and what this
design changes. Experimental and internal testing; off unless a person turns it on for a repository. This design
builds on two features of `main` that the demo branch predates, a worktree's own rules (#340) and rule approval
(#362); the branch is rebased onto `main` before any of it is built.

**Prior art:** [Amp comparison](branch-coordination-amp-comparison.md).

## 1. Summary

When several people and their coding agents work on parallel branches, they remake the same decisions, and they find
the ones they made differently only when the branches merge. Merge Assist makes each decision a **rule** that lives on
the branch, in `.rerail/rules/`, next to the code it governs:

- When a developer corrects their agent ("no, do it this way"), Rerail turns the correction into a rule on their
  branch. The agent follows it for the rest of the branch's work.
- The rule travels with the branch's commits. Merging the pull request makes it a team decision, reviewed in the
  same diff as the code.
- When an agent opens a file, Rerail gives it the decisions other branches made about that file. A compatible one
  from a trusted teammate steers it (**absorb**); one it already breaks, or from someone Rerail cannot vouch for,
  waits for the developer (**review**); one that contradicts this branch's own decision goes to the two people to
  settle (**talk**).
- Changing an earlier decision is an edit to its rule, reviewed like code, never a switch on one machine.

Exact checks (Rerail's engine, `git merge-tree`) decide what they can. The developer's own coding agent CLI, on their
own sign-in, judges only what they cannot. Git is the only shared layer: no server, no Rerail account.

## 2. Problem

- **Learnings are not shared across sessions.** A teammate leaves notes in the repository; another person's agent
  does not read them, or reads them as prose with no way to check its work against them.
- **Discovery is sequential.** The same decisions are remade, and contradictory ones meet at merge time, after both
  sides built on them.
- **Existing tools act after the fact.** Amp's Ship rebases, tests and resolves textual conflicts when a branch lands,
  one branch against `main` ([shipping](https://ampcode.com/docs/orbs/shipping)). Nothing compares two branches that
  are both still in progress, and nothing compares what they decided rather than which lines they changed.

**Intended value:** move coordination left. Re-steering an agent while it works is cheaper than reconciling two
architectures after both land.

## 3. Users and goals

**Primary user:** a developer whose coding agent works on a branch while teammates work on others in the same
repository, with any mix of Claude Code, Codex, Cursor, OMP and Pi.

Goals:

1. A decision made once on a branch holds for the rest of that branch's work without being repeated.
2. A teammate's decision reaches an agent when the agent works on the code it governs, before it builds against it.
3. Real disagreements become a conversation between people, early, with both decisions side by side.
4. Overlapping or adjacent lines alone are never reported as a disagreement.
5. Decisions are reviewed where code is reviewed, and change the way code changes.
6. Nothing leaves the machine that the developer's own tools would not already send.
7. A session never waits for Merge Assist.

Non-goals for this version: resolving conflicts automatically, rebasing, writing to pull requests, blocking a push or
a merge, a hosted service, Git hosts other than GitHub for pull requests.

## 4. Principles

- **Rules are the shared intent.** A decision is a rule document: the same format, checks, approval and settings as
  every other rule. No second format.
- **Git is the sync layer and the source of truth.** Where a decision lives, who made it and how it changed come from
  Git history, never from what a file says about itself. Rerail never commits or pushes.
- **Reviewed where code is reviewed.** A decision enters the team by a merged pull request, and changes the same way.
- **Relevant, not exhaustive.** Work follows what agents touch, not how many branches exist.
- **Deterministic before intelligent.** The engine and Git decide what they can; the judge sees only the rest.
- **Never stronger than steer, never invisible.** Shared decisions steer; they never block. Everything Rerail applies
  automatically is listed, with Undo.
- **People decide disagreements.** Rerail never picks between two developers' decisions.
- **No credentials of its own.** Rerail runs tools the developer already has (`git`, `gh`, their agent CLI) under
  their sign-in.

## 5. The model: a decision is a rule

### 5.1 What a decision is

A decision is one rule document in `.rerail/rules/<name>.json`, the folder Rerail already loads as a checkout's own
rules ([rule packs, "A worktree's own rules"](rule-packs.md#a-worktrees-own-rules)).

| Decision | Rule | Checked by |
| --- | --- | --- |
| One code can break: "callers don't wrap API calls in retry loops" | A `rerail.policy/v1` code rule. `description` is the decision; `scope.include` the folders it governs; `scope.exclude` the places it allows, such as the transport layer the decision says retries belong in | Rerail's engine, exactly |
| Guidance: "API errors are returned as `Result` values" | A conversation rule that matches the agent's tool calls on a path in scope and whose action is the decision's text, sent once per session | The agent, which reads it; the judge, when it is compared with another decision |
| A decision about a diff as a whole | The reserved `llm` clause of a code rule, once it is implemented | The judge |

A decision's rule always steers; Merge Assist never writes one that blocks. A person may commit one in Log for a
transition ([§6.5](#65-changing-a-decision)).

### 5.2 Where a decision lives

Its place comes from Git, relative to the repository's base branch (`origin/HEAD`, else `main` or `master`):

- **Team decision:** a rule in the base branch's tree. It got there through a merged pull request.
- **Branch decision:** a rule this branch added or changed since its merge base with the base branch
  (`git diff --name-status <base>...HEAD -- .rerail/rules`), committed or not.
- **Peer decision:** a rule another live branch added or changed since its own merge base.

A rule's **author** is the author of the commit that last changed its file on that branch (`git log -1`), or, for a
pull request, the author GitHub reports. The **same id on two branches is the same decision**; different content under
one id is an amendment ([§6.5](#65-changing-a-decision)).

A **live branch** is a local branch, a remote-tracking branch (`refs/remotes/origin/*`) or an open pull request that is
not merged into the base, not stacked with this branch, and whose tip is less than 30 days old (a setting).

### 5.3 Lifecycle

```mermaid
stateDiagram-v2
  [*] --> Branch: correction captured, or written by hand
  Branch --> Branch: edited on the branch
  Branch --> Shared: pushed (a remote-tracking branch or pull request)
  Shared --> Team: pull request merged
  Branch --> [*]: file deleted before merging
  Team --> Amended: a branch edits it
  Amended --> Team: that pull request merged
  Team --> [*]: a branch deletes it, merged
```

There is no promotion step and no "rejected" state: merging is promotion, and deleting the file in a pull request is
withdrawal. Both are diffs a reviewer sees.

### 5.4 Trust and approval

Rules Rerail did not publish run in Log until a person approves their content, on this machine, by digest
([rule scoping, "Approval"](rule-scoping.md#approval)). Merge Assist keeps that and adds an approval policy a person
turns on with Merge Assist (recorded, listed on the page, revocable):

| Rule | Approved automatically when |
| --- | --- |
| Written by capture from this person's correction | Rerail wrote these exact bytes for this person |
| A team decision | Its content is in the base branch and the commit that brought it was authored by someone with write access |
| A peer decision | Never by approval: it applies only through absorb or adopt ([§6.4](#64-the-three-tiers)) |

**Trusted** means: a local branch with no pull request behind it from someone without write access, and whose upstream
is the repository's `origin`; a remote-tracking branch whose last commit is by someone with write access; or a pull
request whose author has write access (`gh api repos/{o}/{r}/collaborators/{login}/permission`).

## 6. Experience

### 6.1 Turning it on

Merge Assist is off for every repository. A person turns it on in the dashboard or with `rerail merge-assist on`; a
coding agent cannot. Before the switch, the page says what it does:

- Rerail reads corrections you give your coding agent in this repository and asks **your agent's CLI** (Claude Code
  or Codex) to state each as a decision. That sends the correction and the code it corrected to your agent's
  provider, as your session already does.
- Each decision becomes a rule in `.rerail/rules/`. Commit it to share it; Rerail never commits.
- Rerail reads other branches through Git, and open pull requests through **`gh`**, read-only, with your sign-in.
- Decisions from teammates with write access can steer your agent, and you can undo each one.

It shows what Rerail found: the agent CLI it will use, `gh` state and login, whether Git ignores `.rerail/rules/`
(then decisions stay local), and whether Rerail is set up and started here. It offers one optional line for the
repository's `AGENTS.md`, which a person adds themselves:

```markdown
Team decisions are rules in .rerail/rules/: follow each rule's description where its scope applies.
```

With it, agents without Rerail follow merged decisions too, as guidance.

### 6.2 Capturing a decision

1. **At each turn end**, a free filter (the correction phrases `rerail rules evidence` recognises) marks corrections.
   The agent gets an immediate acknowledgement, with no model call: *"Rerail will save this as a decision when you
   commit."*
2. **At a batch point** (the agent's `git commit` tool call, the session going idle, or the person asking "save that
   as a decision"), the judge reads the marked corrections with the code before and after each and the decisions
   already in force, and answers one of:

   | Outcome | What Rerail does |
   | --- | --- |
   | One-off fix | Nothing |
   | Restates a decision in force | Nothing; it may tighten that rule's scope |
   | New decision | Writes a rule |
   | Contradicts a team decision | Writes an amendment of that rule ([§6.5](#65-changing-a-decision)) |

3. **A code rule is validated before it is kept:** it compiles; it catches the code the agent wrote before the
   correction and passes the code after; and it matches nothing in the checkout as it is. A failure goes back to the
   judge once, with the concrete failure (compile error, failing case, or up to three matches as `path:line: text`);
   a match in a place the decision allows becomes a `scope.exclude` entry rather than a narrower scope. A rule that
   still fails is kept as guidance, with the reason.
4. Rerail writes the file and tells the agent, which tells the person in one line and includes the file in the commit
   it is making:

```text
Rerail saved your correction as a decision for this branch: "Retries live in the transport layer; callers don't
wrap requests in retry loops." It is in .rerail/rules/retries-in-transport.json; include it in this commit.
```

### 6.3 Decisions when they are relevant

When the agent reads or edits a file, Rerail looks its path up in the index ([§7.2](#72-the-decision-index)) and
gives the agent, once per session, the decisions in force whose scope covers it: team decisions, this branch's, and
absorbed peer decisions. Code rules among them already steer the way every rule does; guidance goes out under the
existing "Rerail has guidance for this step" lead. A decision without a scope covers the repository and is given once
at session start.

The same lookup drives comparison: a peer decision is compared with this branch's only when an agent here works in
its scope. A branch nobody's work touches costs nothing.

A person can see everything at once with **Scan now** on the page, which compares every live branch.

### 6.4 The three tiers

For a peer decision relevant to this checkout:

| Tier | When | What happens |
| --- | --- | --- |
| Absorb | Trusted, new (not an amendment of a team decision), this branch's changes pass its code rule, and it does not contradict a decision in force here | It applies in this checkout: a code rule steers, guidance is given when relevant. Listed under **Added from other branches**, with Undo |
| Review | This branch already breaks it; or it is untrusted; or it amends a team decision; or the judge cannot tell | Listed under **Needs your review** with the evidence. **Adopt** copies the rule into this branch's `.rerail/rules/`, so it is reviewed with this branch's pull request; **Dismiss** hides it until its content changes |
| Talk | It contradicts a decision in force here (this branch's, or a team decision), and its author is trusted | Listed first under **Decided differently**: both decisions, the author and branch or pull request, the judge's explanation, files `git merge-tree` reports conflicting in the scope, and a message to send. Nothing is applied |

An absorbed decision is judged again whenever either side changes; it moves to Review or Talk when trust, the exact
check or the comparison stops allowing it. An untrusted author's contradiction is a Review item, and its text never
reaches the agent.

Contradictions between two code rules are exact: each side's rule runs against the other side's changes in its scope.
Two branches editing the same rule file differently is exact too (`git merge-tree` on `.rerail/rules/`). The judge
compares only guidance, and only pairs whose scopes overlap.

### 6.5 Changing a decision

A design change is an edit to the decision's rule on a branch, not a setting:

| What the developer means | How | Who sees it |
| --- | --- | --- |
| "Right, but not here" | Add the path to `scope.exclude`, or a suppression comment where the rule honours them | Everyone, in the diff |
| "We are moving away from this" | Set `"mode": "log"` in the file: violations are still recorded, which measures the migration, and it stops steering | Everyone |
| "This decision is wrong now" | Edit the rule and raise its `revision`, replace it, or delete the file | Everyone; a reviewed change to a team decision |
| Trying something locally | The checkout's own Log or off setting | Only this person |

The usual path is capture: the developer contradicts a team decision in a session ("no, we're switching to
exceptions"), and capture writes an **amendment**, the same file with the new decision and `revision + 1`, instead of
a second, contradictory rule. This branch follows the new version at once. The note says:

```text
This changes a team decision by Jeremy Barnes, merged in #412. Your branch carries the change; branches that rely on
it will see it for review.
```

An amendment is never absorbed. Its original author sees it under **Decided differently**; other live branches see it
under **Needs your review**, with the places their code follows the old version and breaks the new one, from the exact
check. When the pull request merges, every checkout loads the new version, approved again under the policy of
[§5.4](#54-trust-and-approval). Undo is reverting the commit.

Turning a team decision to Log or off in one checkout asks once: *"Propose changing this decision for the team?"* Yes
writes the amendment; the setting itself is never shared.

### 6.6 At landing

The agent's `gh pr create` and `git push`, and **Check now**, run the expensive exact work once:

- this branch's changes against every decision in force and every live peer's code rules, and theirs against this
  branch's, in scope;
- `git merge-tree` against each live peer that touches the same scopes;
- the repository's own checks are not run.

A result counts only when it is new against the base branch: a rule the base already breaks is not this branch's
finding. Findings go to the session as one note and to the page. Nothing blocks.

### 6.7 The dashboard page

**Merge Assist**, under Experimental in the sidebar, for the selected checkout:

- **Status strip:** on or off; branch and base; the agent CLI used for judgment; `gh` state and login; how current the
  index is (*"remote branches as of the last fetch, 12 min ago"*); what waits in the queue (*"comparing 2 new
  decisions"*); judgment calls this hour against the budget; **Check now**; Turn off.
- **Decided differently**, **Needs your review**, **Added from other branches** (Undo), **Team decisions** (from the
  base branch, each with its author and the pull request that merged it, and **Propose a change**), and **This
  branch's decisions** (added or amended here; committed or not; Delete).
- A notice when automatic approvals happened, with each approval listed and revocable.
- Every string another person wrote is text, never markup; a pull request links only to its GitHub URL.

Merge Assist rules do not appear under **Created by you**; team and branch decisions appear with the checkout's own
rules under **This worktree**, where every rule setting works on them.

### 6.8 CLI and MCP

```sh
rerail merge-assist [status] [--json]        # this checkout: state, decisions, findings
rerail merge-assist on | off                 # people only on the dashboard and API
rerail merge-assist check [--json]           # the landing checks, now
rerail merge-assist find <path>              # decisions and live sessions or branches touching a path
rerail merge-assist adopt | dismiss | undo <finding-id>
```

MCP tools carry the same operations: `merge_assist_get`, `merge_assist_check`, `merge_assist_find` (an agent may ask
which decisions and branches touch a file before it starts), `merge_assist_finding_set`, and the people-only
`merge_assist_settings_set`. An agent may dismiss an open finding when the person asks; it may not adopt, undo, or
turn Merge Assist on. The CLI runs as the person, so an agent with a shell can run any of it, as it can any Rerail
command; the people-only guarantee covers the MCP and HTTP API.

### 6.9 Notes the agent reads

Each starts by saying it comes from Rerail, not the user; another person's text appears only quoted, escaped, and
labelled with its author.

| When | What the agent reads |
| --- | --- |
| A correction is marked | Rerail will save it as a decision at the next commit |
| A decision is saved | The decision, its file, and to include it in the commit |
| A file is opened that a decision in force covers (guidance only, once per session) | The decision, whose it is, and to follow it where it applies |
| Talk | Who decided differently, on which branch, both decisions; tell the person and keep the current approach until they decide |
| Review | How many decisions wait for the person's review; mention it, apply none |
| Amendment saved | That it changes a team decision, whose, and that other branches will review it |
| Landing check found something | The findings, one line each |

## 7. Architecture

```mermaid
flowchart LR
  subgraph git["Git, the shared layer"]
    RR[".rerail/rules/ on every branch"]
  end
  REFS["refs, packed-refs, worktree HEADs"] -->|watch| W
  GH["gh: pull request heads, authors, trust"] -->|poll, cached by head commit| W
  EV["session events: reads, edits, corrections"] --> Q
  RR --> W
  W["Worker: one per machine"] --> IX["Decision index, per repository, keyed by Git object id"]
  IX --> LK["Path lookup: scope globs to decisions, paths to sessions"]
  LK -->|overlapping pairs| Q["Job queue in SQLite"]
  Q --> W
  W --> J["Judge: one CLI run at a time, cached per pair"]
  W --> X["Exact checks, cached per rule digest and diff"]
  J --> V["Views: a pure function of the index, caches and the person's choices"]
  X --> V
  V --> OUT["Rule activation, notes, dashboard, CLI, MCP"]
```

### 7.1 The worker

One `rerail merge-assist worker` per machine, supervised by the daemon, does all slow work: Git, `gh`, the judge and
exact checks, with asynchronous processes only. Hooks and the daemon enqueue jobs and read results from SQLite; a hook
never waits on Merge Assist. The worker keeps no state of its own, so a restart loses nothing. While it is down the
page says so and nothing blocks. A process rather than worker threads, because the judge and the engine can each use a
lot of memory.

### 7.2 The decision index

Per repository, shared by its checkouts:

- **Refresh on change only.** A watch on `refs/`, `packed-refs` and `worktrees/*/HEAD` (with a slow timer as a
  safety net) marks branches whose tip moved. One `git for-each-ref` lists tips; one
  `git cat-file --batch-check` resolves `<tip>:.rerail/rules` for every moved branch at once. A branch whose folder
  has the same tree id as the base adds nothing and costs nothing.
- **Keyed by content.** Each rule is indexed by its blob id, with the branches that carry it, its place
  ([§5.2](#52-where-a-decision-lives)), author and trust. Parsed and compiled once per blob.
- **Remote work through Git first.** Remote-tracking branches are already on the machine, so a pushed branch is
  visible without a pull request and without a network call. `gh` adds pull request numbers, titles and author
  permissions, and is asked for file content only when a head commit is not local. Its answers are cached by head
  commit, which never changes.
- **Scopes as one matcher.** Every indexed rule's `scope.include` and `scope.exclude` compile into one linear-time
  matcher per repository, so a path lookup costs microseconds.

### 7.3 The file-touch index

From the `events` table, which already records every tool call: path → sessions, branches and times. It gives
`merge_assist_find`, overlap between live sessions on this machine including uncommitted work, and, when two live
sessions edit the same files, one note suggesting they coordinate, following Amp's advice to keep same-file work in
one thread.

### 7.4 Caches and filters

- **Comparisons** are cached per unordered pair of rule blob ids plus the prompt version. A pair is judged only when
  both are guidance (code rules compare exactly), their scopes intersect, and an agent works in that intersection;
  the bundled embedding model orders the queue by similarity. Pairs are batched into one call.
- **Exact checks** are cached per rule digest plus a hash of the branch's changed files in the rule's scope.
- **Capture** answers are cached by their input; a repair round has its own key.

### 7.5 The judge

The developer's Claude Code or Codex CLI, run headless with no tools, MCP servers included, in an empty `0700`
temporary folder, with a JSON schema, a 120-second timeout and a 1 MiB output cap. The invocations are those of the
v1 build ([v1 §9.4](merge-assist-v1-build.md#94-agent-cli-invocations)), each checked against a real run. OMP and Pi
are not judges: their MCP servers stay connected in print mode. At most one run at a time per machine, queued;
40 runs an hour per repository by default. A comparison's schema names only the ids it was given.

### 7.6 Views

Tiers, findings and the page's status are a function of the index, the caches and the person's choices (adopt,
dismiss, undo). `status()` is one database read; the dashboard's poll runs no process.

### 7.7 Applying decisions

- **Team and branch decisions** are the checkout's own rules, loaded by the existing worktree pack from the working
  tree; nothing extra is needed.
- **Absorbed peer decisions** load as one read-only pack per peer branch, built from its Git tree and cached by tree
  id, present in this checkout only, in Steer at most. A rule id already in force here wins over a peer's: same id,
  different content is an amendment, which is never absorbed.
- **Adopted** peer decisions are copied into this branch's folder and become branch decisions.

## 8. Security

Merge Assist reads what other people wrote (rule files, pull request metadata, branch names, logins) and passes some
of it to the judge and, when applied, into the developer's agent session.

| Threat | Mitigation |
| --- | --- |
| A rule file injects instructions into the developer's agent | Peer rules apply only from trusted authors, else wait for review; nothing above Steer; another person's text is quoted, escaped and labelled with its author; an untrusted author's text never reaches the agent |
| A rule file injects instructions into the judge | No tools and no MCP servers; per-call nonce fences around all data; JSON schema; the answer is only a verdict |
| A branch rewrites a team decision to widen what an agent may do | Changes to team decisions are amendments, never absorbed, always reviewed |
| Self-declared authorship | Authors come from Git commits and GitHub, never from the file |
| A local copy of an outsider's pull request passes as one's own branch | A local branch that holds an untrusted pull request's commits, or tracks a remote other than `origin`, inherits its distrust |
| Costly or malformed scopes, regexes, files | The worktree pack's checks (regular files, size, valid document, compile); scope globs capped at four wildcards and matched in linear time; per-branch count limits |
| An agent widening its own latitude through MCP | Turning on, adopting, undoing and approval are people's operations; an agent may only dismiss an open finding |
| An enterprise `gh` token sent to a look-alike host | `GH_ENTERPRISE_TOKEN` and `GITHUB_ENTERPRISE_TOKEN` are stripped; a non-github.com host is read only through a stored `gh` login for it |
| Runaway spending of the developer's tokens | Path relevance, pair caching, one run at a time, an hourly budget shown on the page |

The v1 build's two security and correctness reviews, and their fixes, carry over
([v1 §7](merge-assist-v1-build.md#7-security)).

## 9. Privacy and data

**What leaves the machine:**

| Path | What | To whom |
| --- | --- | --- |
| Capture | The marked corrections; summaries of the calls before them; up to four edits with the code before and after (2,000 characters a side); the decisions in force | The developer's agent CLI provider, under their sign-in |
| Comparison | Pairs of decision texts and scopes; names of conflicting files | The same |
| `gh` | Repository name, commit ids, logins; read-only | GitHub |
| Committed rules | A decision's text, scope, check and mode | The Git host, when the developer pushes |

Never sent: the transcript, the session's first prompt, agent replies, tool output, files under secret-looking paths,
files outside the checkout. A committed decision is the judge's wording of a correction and can name identifiers or
files.

**What stays on the machine** (`~/.rerail/rerail.sqlite`, readable by the user only): the index, job queue, judgment
cache (whose capture answers hold snippets of code), comparisons, exact-check results, findings, the person's choices
and approvals. Cached entries for content no live branch holds are removed; judgments expire after 30 days; per-session
entries go with their session; `rerail reset` and turning Merge Assist off remove everything for the repository.

For a session to stay entirely local, Merge Assist stays off: capture needs a model, and Rerail uses only the
developer's agent CLI. A local-model judge (Codex with `--oss`) is an open question.

## 10. Performance and scale budgets

| Measure | Budget |
| --- | --- |
| Time Merge Assist adds to any hook | 0 (enqueue only) |
| `status()` | One database read; no process |
| Path lookup on a read or edit | Under 1 ms for 1,000 indexed rules |
| Index refresh | Proportional to branches whose tip moved; nothing for unchanged ones |
| Judge runs | Proportional to new overlapping guidance pairs an agent works in, plus captures; one at a time |
| `gh` | Pull request list on a slow poll; content only for heads not local; cached by head commit |
| Target repository | 150 branches, 20 open pull requests, 10 live sessions, 200 decisions |

## 11. Prior art

From Amp ([comparison](branch-coordination-amp-comparison.md)), Merge Assist borrows:

- **Guidance in the repository, loaded when relevant.** `AGENTS.md` files and their glob-scoped mentions load when
  the agent reads a matching file ([AGENTS.md](https://ampcode.com/docs/customize/agents-md)). Merge Assist's path
  lookup does the same with rules, and offers the `AGENTS.md` line so agents without Rerail follow merged decisions.
- **Which threads touched a file.** `find_thread` by file ([Find Threads](https://ampcode.com/news/find-threads))
  becomes the file-touch index and `merge_assist_find`.
- **Exact checks once, at landing, counted against the base.** Ship ignores a failure that also fails on the base
  branch ([shipping](https://ampcode.com/docs/orbs/shipping)).
- **Ask a person only about substantive conflicts.**

It differs from Amp where Amp has no answer: it compares branches that are all still in progress, it compares
decisions rather than lines, it works across coding agents, and its shared layer is Git rather than a hosted index.

## 12. Alternatives considered

| Alternative | Why not |
| --- | --- |
| A landing gate only | Finds conflicts after both sides are built. Kept as [§6.6](#66-at-landing) |
| A radar comparing every pair of branches continuously | Cost grows with branches squared; noisy. Replaced by path relevance |
| Intent claims declared at plan time, published in pull request bodies | Plans are vague, many pull request bodies are empty, and it writes to GitHub. Corrections are firmer evidence of a decision |
| Merging branches speculatively and type-checking the result | Exact but blind to design; costly per pair. Its baseline rule survives in the landing checks |
| A separate decision format (the v1 `rerail.candidate/v1` files) | Duplicates rules, needs its own activation and trust, and is not reviewable as a rule |
| Changing a decision by switching its rule to Log locally | Invisible to everyone else; the decision never changes for the team |
| A central server holding the index | Breaks local-first; Git already carries the decisions |
| Fetching from remotes in the background | Breaks "no remote Git operations" and moves the user's refs. Remote-tracking refs plus read-only `gh` suffice |

## 13. Migration from the demo build

- `.rerail/candidates/*.json` files are converted once into rules in `.rerail/rules/` (the candidate's `rule`, or a
  guidance rule from its decision), in the working tree, for the person to commit.
- The v1 activation (library rules with `~tag` ids, repository-off and checkout-on settings, explicit offs in nested
  checkouts) is removed on first start, as v1 itself removes its rules when turned off.
- v1's tables are replaced by the index, queue and caches; v1 findings are re-derived.

## 14. Build status

| Part | v1 demo build | This design |
| --- | --- | --- |
| Decision format | `rerail.candidate/v1` in `.rerail/candidates/` | Rules in `.rerail/rules/` |
| Own versus team versus peer | Every file in the checkout counts as own | From Git history against the base |
| Applying decisions | Library rules with per-checkout settings | Worktree pack; a read-only pack per absorbed peer |
| When work happens | Scan at every session start and after each save | Index refresh on ref changes; comparisons when an agent works in scope |
| Where it runs | In the daemon, with synchronous Git | A worker process, asynchronous |
| Remote work | Open pull requests through `gh` only | Remote-tracking branches first, `gh` for metadata |
| Comparisons | One judge call per peer; any new decision invalidates all | Per pair of rule contents; exact for code rules |
| Capture | At every turn end with a correction | Marked at turn end, judged in a batch at commit or idle; amendments |
| Changing a decision | Not modelled | Amendments, reviewed |
| Exact checks | At each scan | At landing and on demand, counted against the base |
| Approval | Not used (library rules) | Rerail's approval, with an opt-in policy |
| Built and carried over | Judge runner and prompts with fences and repair; `gh` client; capture validation; tier model and its tests; notes' framing and escaping; agent guards; dashboard page structure | |

## 15. Plan and demo

Order of work:

0. Rebase the demo branch onto `main`, which brings a worktree's own rules and rule approval.
1. Decisions as rules in `.rerail/rules/`, with place and author from Git history; team decisions on the page.
2. The index, with remote-tracking branches and the refs watch.
3. Path relevance: decisions given when a file is opened; comparisons scoped to it.
4. Amendments.
5. The worker process and job queue.
6. Pair caches, batched capture, landing checks.
7. The approval policy, retention, and the v1 migration.

The demo, on two machines:

1. Jeremy, on `jbarnes/session-retry`, corrects his agent: "don't wrap API calls in retry loops; retries belong in the
   transport". Rerail saves a code rule in `.rerail/rules/`; his agent commits it with the change and he pushes.
2. Tim's machine fetches. When Tim's agent opens `src/api/users.ts`, the rule is absorbed; when it writes a retry
   loop, Rerail steers it with Jeremy's rule.
3. Jeremy also decided "API errors are returned as `Result` values". Tim tells his agent the opposite; capture sees a
   contradiction with Jeremy's branch, and the page shows **Decided differently** with a message to send.
4. After Jeremy's pull request merges, Tim says "no, we're switching to exceptions": capture writes an amendment of the
   team decision, and Jeremy's next branch sees it for review.

## 16. Open questions

- Should the approval policy for team decisions be on by default once Merge Assist is on?
- A short `rationale` field in rule documents, so reviewers read why and not only what?
- Guidance needs conversation rules that match tool calls on a path out of internal testing; is that the next step for
  them, or should guidance be its own rule kind?
- Should reviewers' pull request comments be a source of decisions?
- Is a local-model judge worth supporting for teams that cannot send code to a provider?
- How should decisions that span repositories (a shared library and its callers) be scoped?
