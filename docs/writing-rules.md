# Writing rules for your repository

A Rerail rule checks a coding agent's work while it works. When the work breaks the rule, Rerail tells the agent ("steers" it) in
the session, and the agent fixes it. This page covers how to get good rules for your own repository: which to write, how to write
and test them, and how to fix one that misfires.

The short version is to type `/rerail-create` in your coding agent (`$rerail-create` in Codex).

| You type | What happens |
|---|---|
| `/rerail-create` | The agent suggests rules from this repo's sessions and written standards, and writes the ones you pick |
| `/rerail-create use our logger, not console.log` | The agent writes that rule |
| `/rerail-create the no-sync rule keeps telling you to use fs/promises in our CLI` | The agent tunes that rule, or offers to switch it to Log |

Every path ends the same way. The agent tests the draft, reads it back to you, and adds it only after you say yes. The agent
follows the skill in [`skills/rerail-create/`](../skills/rerail-create/SKILL.md), which ships in the package. Its references are
the detailed guide to everything on this page.

## Which rules are worth having

Rerail measured its rules on 69 real pull requests, each replayed with steering off and on.

**What paid off:**
- **The repo's own conventions.** Rules taken from a repository's own conventions or docs cut the violations left at the end to
  zero, for 1% to 15% more cost.
- **Cheap defects.** Defects fixed on the first steer, such as an `async` function that never awaits, paid for themselves.

**What did not:**
- **Generic style rules that clash with the repo.** They were steered again and again, and none of their fixes made the code more
  correct.
- **Rules that go against the repo's convention.** Where a rule did, quality fell and cost rose by half.

A rule is worth writing when:

- you have corrected the agent on it, or written it down as a standard;
- the repo already follows it almost everywhere;
- following the steer can never break working code;
- nothing else (a linter, formatter or type-checker) enforces it.

[Suggesting rules](../skills/rerail-create/references/suggesting-rules.md) has the full scoring.

### Evidence from your sessions

```sh
rerail rules evidence [--repo <dir>] [--days <n>] [--limit <n>] [--json]
```

This reads what Rerail recorded for this repository, across every coding agent and every worktree, over the last 30 days. It
changes nothing.

| It lists | Which can become |
|---|---|
| Your corrections, with what the agent had just done | A code rule, or a command rule |
| Risky commands the agent ran: force pushes, skipped hooks, hard resets, recursive deletes, publishes | A command rule |
| How often each rule steered | A rule to tune, not a new one |
| The files where the repo writes its standards (CLAUDE.md, AGENTS.md, CONTRIBUTING.md, linter configs, Cursor and Copilot instructions) | Rules from the standards they state |

```text
● acme-web: 42 sessions in the last 30 days (claude-code 30, codex 12), 310 prompts.
  Corrections: 9.
    2026-10-12 claude-code [standing instruction] Don't use console.log, we use the logger in src/log.ts
      after: Edit src/api/orders.ts
  Risky commands:
    force push ×2  git push --force origin feature/x
  Steers by rule:
    n-no-sync ×12
  Standards files: AGENTS.md, CONTRIBUTING.md, eslint.config.js
```

## Two kinds of rule

**A code rule** checks the files the agent edits: imports across a layer boundary, a call that bypasses a helper, a blocking call
in async code. It is a `rerail.policy/v1` document with a scope (globs) and clauses (a `regex`, or a syntax query over Python,
JavaScript, TypeScript, PHP or Blade). See [Writing a code rule](../skills/rerail-create/references/code-rules.md).

**A command rule** checks the commands the agent runs: force pushes, `--no-verify`, publishing. It is a conversation rule with a
structural `match` over `input.command`. See [Writing a command rule](../skills/rerail-create/references/conversation-rules.md)
and, for regex technique, [tool-call-rules.md](tool-call-rules.md).

## Testing a draft

A draft is the rule plus the cases it must catch and let through:

```json
{ "kind": "code", "rule": { "…": "…" }, "cases": { "caught": [{ "path": "src/a.ts", "content": "…" }], "passed": [] } }
```

```sh
rerail rules test draft.json            # validate, run the cases, count matches across the repo; writes nothing
rerail rules add draft.json             # store it in your library and add it, on, to this repo
rerail rules export --output rules.json # your rules, to read or edit one
```

**Before adding a draft, check:**
- **Cases.** Every case comes out right, including a near miss: code that looks like a violation but is not.
- **Repo matches.** Each match in the repo is a real violation.
- **Size.** It matches a handful of places, not hundreds. Hundreds mean the repo's convention says otherwise.
- **Message.** Following it cannot break working code.

[Rule quality](../skills/rerail-create/references/rule-quality.md) has the gates, the five kinds of harmful steering, and
the fix ladder for a rule that misfires.

## Steer or Log

- **Steer (`steer`)** tells the agent in the session. Use it for a rule you are sure of.
- **Log (`log`)** keeps a record and tells no one. Use it for a rule you are trying out, or one the existing code breaks too
  often to steer on.

A rule from `/rerail-create` is never `block`, and belongs to the repository it was made in.

### Observing unfinished task reports

The experimental conversation-rule draft in
[`fixtures/host/task-completion/rule-draft.json`](../fixtures/host/task-completion/rule-draft.json)
records direct admissions such as "I haven't finished your request", "The implementation remains incomplete",
or "Not yet done from the spec". Generic "Still to do" headings and "Next I'll run" promises are insufficient.
It uses a structural text match at the assistant's `turn` boundary, in **Log** mode, with `repeat.mode: once`.
It is opt-in and is not attached to repositories automatically. From a source checkout:

```sh
rerail rules test fixtures/host/task-completion/rule-draft.json --json
rerail rules add fixtures/host/task-completion/rule-draft.json
```

Log records matches without sending the action to the agent. The draft includes shortened examples from internal
Haiku runs, other Rerail sessions and an Opus run, plus constructed phrasing, completion, blocker, quotation, source,
and boundary cases; the catalog regression test runs them through the engine.
The wording detects possible unfinished work, not actual task completion. It can match quoted text, optional work,
or historical admissions, and it misses incomplete work reported as "Done". A blocker and an admission can also
appear together. The action therefore asks the agent to check the original request and continue only within its
existing authorization, respecting missing information, necessary approvals, and user stop instructions.

Keep this experiment in Log while measuring its matches. If later changed to Steer, the existing OMP adapter has a
late-delivery continuation path; Claude Code and Codex hold stop-time steers until the next context event. A `turn`
boundary is host-dependent: OMP emits one after each model call, so this is not a portable final-stop detector.
Once-per-session delivery is not once per user request, and a successful match test does not verify host continuation.

#### Cross-session review (22 October 2026)

The wording review screened 828 nonempty turn-end messages from 105 local Rerail sessions (633 OMP, 118 Claude Code,
77 Codex), excluding the rule-design session. It also screened complete final transcript messages from 360 non-Haiku
Python study runs (90 each for DeepSeek, Luna, Opus, and Qwen), separately from the original 90 Haiku runs.
An extended screen read 3,414 transcript files from the older witness, Stage 1, Stage 2, steer-strategy and packs
collections, retaining 3,305 unique sessions with nonempty normal-stop endings. Those include task and CI sessions
and voided attempts; they are not an independent benchmark cohort. Most (2,749) used Opus; the rest used Luna,
Command A+, DeepSeek, GLM, Mistral, Qwen and North Mini Code.
Candidate messages and their surrounding requests were inspected; the entire corpus was not independently labelled
for task completion. Counts below are text matches before repeat suppression, not precision or recall estimates.

| Source | Earlier wording | Revised wording |
|---|---:|---:|
| Local sessions, 828 messages | 3 | 0 |
| Non-Haiku study runs, 360 endings | 1 | 0 |
| Older study collections, 3,305 session endings | 11 | 0 |
| Haiku study runs, 90 endings | 10 | 4 |

The earlier three local matches were all from one study session. They reported work waiting on running batches,
or a blocker owned by another session. The Opus match handed off a `tox` check because the environment lacked its
tools. These are poor reasons to automatically resume an agent; all four now pass and have fixture excerpts.
Local event sequences 56860, 59344, and 63465 identify those status reports; the Opus example is the legacy-message
Falcon #2723 on-1 run. Only relevant excerpts are included, without unrelated session details.

The older-study matches include browser tests blocked by Playwright downloads (Rocicorp #6706), migrations waiting
on a background Composer install (Krayin #2575), and desktop verification that could not run in the sandbox
(Openscreen #963). They also include ambiguous verification and repo-workflow handoffs, which need request context
to judge; the 11 matches should not all be labelled false positives. Additional fixture excerpts cover background
dependency waiting, browser availability and a generated-code getter that throws "not implemented".

The revised wording retains four clear Haiku admissions: Hive current on-2, Inspect current on-3, Inspect legacy
on-1, and Inspect off-1. It also covers constructed variations about an unfinished request or implementation rather
than naming the actual bug. It intentionally misses the OpenEvolve promise to run a smoke check and the arXiv
promise to extract another helper. Those endings can indicate unfinished work, but a bare future promise cannot
distinguish it from optional, blocked, or background work. There were no additional confirmed positive examples in
the non-Haiku sample, so broader recall remains unproven. Keep the rule in Log.
