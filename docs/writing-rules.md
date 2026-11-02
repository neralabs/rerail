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
in async code, an old path still written in a `Justfile`, a file in the wrong folder. It is a `rerail.policy/v1` document with a
scope (globs) and clauses (a `regex` over a file's text or path, a `file` query, or a syntax query over Python, JavaScript,
TypeScript, PHP or Blade). See [Writing a code rule](../skills/rerail-create/references/code-rules.md), and for rules over
text and paths in any file, [Writing path and text rules](path-and-text-rules.md).

**A command rule** checks the commands the agent runs: force pushes, `--no-verify`, publishing. It is a conversation rule with a
structural `match` over `input.command`. See [Writing a command rule](../skills/rerail-create/references/conversation-rules.md)
and, for regex technique, [tool-call-rules.md](tool-call-rules.md).

## Testing a draft

A draft is the rule plus the cases it must catch and let through. It is not the bare rule: `rerail rules test` and
`rerail rules add` read this envelope, and `rerail rules test --help` prints it.

```json
{
  "kind": "code",
  "rule": { "format": "rerail.policy/v1", "id": "local:…", "revision": 1, "mode": "steer", "description": "…", "scope": { "include": ["…"], "exclude": [] }, "clauses": ["…"] },
  "cases": {
    "caught": [{ "path": "src/a.ts", "content": "…" }],
    "passed": [{ "path": "src/a.ts", "content": "…" }]
  }
}
```

| Field | What it holds |
|---|---|
| `kind` | `code` |
| `rule` | the rule document |
| `folder` | optional: the repository folder to add the rule at, instead of the whole repository |
| `cases.caught`, `cases.passed` | files the rule must catch, and files it must let pass: `{ "path", "content" }` |

[Writing path and text rules](path-and-text-rules.md#text-in-files) has complete drafts to start from.

```sh
rerail rules test draft.json            # validate, run the cases, list the matches across the repo; writes nothing
rerail rules test draft.json --json     # the same, listing every match and every file the rule could not check
rerail rules add draft.json             # store it in your library and add it, on, to this repo
rerail rules export --output rules.json # your rules, to read or edit one
```

The text report shows the first five matches and the count; `--json` lists them all. A match on a file's path, or of a
`file` query, names the file with no line. The report also counts the binary files in the rule's scope, which no rule
reads.

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

## Steer or Log

- **Steer (`steer`)** tells the agent in the session. Use it for a rule you are sure of.
- **Log (`log`)** keeps a record and tells no one. Use it for a rule you are trying out, or one the existing code breaks too
  often to steer on.

A rule from `/rerail-create` is never `block`, and belongs to the repository it was made in.
