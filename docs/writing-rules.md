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

Every path ends the same way. The agent tests the draft, reads it back to you, and adds it only after you say yes. The rule
then waits for your approval: the agent audits what it added, tells you what it found, and you decide. The agent follows the
skill in [`skills/rerail-create/`](../skills/rerail-create/SKILL.md), which ships in the package. Its references are the
detailed guide to everything on this page.

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
rerail rules add draft.json             # store it in your library and add it to this repo; it runs once you approve it
rerail rules list                       # the rules that wait for your approval
rerail rules approve <id>               # print the content that waits, then approve it
rerail rules export --output rules.json # your rules, to read or edit one
```

The repo pass never reads hidden files and folders, files whose names usually hold secrets (`*.pem`, `id_rsa*`, `.ssh/`,
…), or files Git ignores unless you turned the repository's Follow .gitignore setting off. It counts how many it left
out, without names. See [Reference](reference.md#scan-and-settings-controls) for the full list.

**Before adding a draft, check:**
- **Cases.** Every case comes out right, including a near miss: code that looks like a violation but is not.
- **Repo matches.** Each match in the repo is a real violation.
- **Size.** It matches a handful of places, not hundreds. Hundreds mean the repo's convention says otherwise.
- **Message.** Following it cannot break working code.

[Rule quality](../skills/rerail-create/references/rule-quality.md) has the gates, the five kinds of harmful steering, and
the fix ladder for a rule that misfires.

## Approving a rule

A rule runs only with content a person approved. Content is the whole rule except its `mode` and `revision`, so changing
only those needs no approval.

- **Nera's rules need none.** The rules of packs installed from the npm registry (by set-up, or `rerail packs add <id>`) run
  as shipped.
- **What you save in the dashboard is approved as you save it.**
- **Everything else waits for your approval:**
  - a new rule created by you that `/rerail-create`, `rerail rules add`, `rerail rules import` or a coding agent stores, and
    every change to such a rule's content;
  - the rules of a pack added from a folder or a tarball (`rerail packs add <folder>`), and every rule an update of such a
    pack adds or changes.

While a rule waits:

- **A new rule runs nowhere.** Where it is added, the repository lists it as unavailable, awaiting approval.
- **A changed rule keeps running the content you approved last.** The change runs once you approve it.
- **A coding agent's edited copy of a By Rerail rule** is added where the original is, but the original stays on there. The
  original is switched off only when you approve the copy.

To approve a rule:

- **In the dashboard**, which shows the rules that wait.
- **With `rerail rules approve <id>`.** It prints the content that waits, and the approved content it replaces, then approves
  it. It asks nothing itself. Rerail does not pre-approve it in Claude Code, Cursor or Codex, so they ask you before a coding
  agent runs it. `rerail rules list` shows what waits.

A coding agent cannot approve a rule through Rerail's tools: the MCP tool `rule_approvals_approve` refuses it. Before it asks
you, `/rerail-create` audits the rule and gives a verdict. It flags text that tells the agent more than the rule, commands or
URLs, hidden or encoded text, mentions of secrets, anything that weakens other rules or Rerail, and anything unrelated to
the rule's purpose.

An approval holds for the content you read: when the rule changed after you read it, the approval is refused and you read it
again. Upgrading to a Rerail with approval approved every rule created by you as it stood, so nothing that ran before stops.

## Steer or Log

- **Steer (`steer`)** tells the agent in the session. Use it for a rule you are sure of.
- **Log (`log`)** keeps a record and tells no one. Use it for a rule you are trying out, or one the existing code breaks too
  often to steer on.

A rule from `/rerail-create` is never `block`, and belongs to the repository it was made in.
