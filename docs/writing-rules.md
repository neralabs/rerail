# Writing rules for your repository

A Rerail rule checks a coding agent's work as it goes. When the work breaks a rule, Rerail tells the agent in the session
("steers" it), and the agent fixes it.

Start with `/rerail-create` in your coding agent (`$rerail-create` in Codex):

| You type | What happens |
|---|---|
| `/rerail-create` | The agent suggests rules from this repo's sessions and written standards, and writes the ones you pick |
| `/rerail-create use our logger, not console.log` | The agent writes that rule |
| `/rerail-create the no-sync rule keeps telling you to use fs/promises in our CLI` | The agent tunes that rule, or offers to switch it to Log |

The agent tests each draft, reads it back, and adds it only after you say yes.

<details>
<summary>Where the agent's method comes from</summary>

Every path ends the same way: test, read back, add on your yes. The agent follows the skill in
[`skills/rerail-create/`](../skills/rerail-create/SKILL.md), which ships in the package. The skill's references are the
detailed guide to everything on this page: which rules to write, how to write and test them, and how to fix one that
misfires.

</details>

## Which rules are worth having

Rules from your repo's own conventions pay off; generic style rules do not.

A rule is worth writing when:

- you have corrected the agent on it, or written it down as a standard;
- the repo already follows it almost everywhere;
- following the steer can never break working code;
- nothing else (a linter, formatter or type-checker) enforces it.

<details>
<summary>How Rerail measured this</summary>

Rerail replayed 69 real pull requests with steering off and on:

| Rule type | Result |
|---|---|
| **The repo's own conventions or docs** | Violations left at the end fell to zero, for 1% to 15% more cost |
| **Cheap defects** fixed on the first steer (an `async` function that never awaits) | Paid for themselves |
| **Generic style rules that clash with the repo** | Steered again and again; no fix made the code more correct |
| **Rules that go against the repo's convention** | Quality fell and cost rose by half |

</details>

[Suggesting rules](../skills/rerail-create/references/suggesting-rules.md) has the full scoring.

### Evidence from your sessions

`rerail rules evidence` lists what Rerail recorded for the repository that could become rules. It changes nothing.

```sh
rerail rules evidence [--repo <dir>] [--days <n>] [--limit <n>] [--json]
```

<details>
<summary>What it lists, with sample output</summary>

It covers the last 30 days, across every coding agent and every worktree of the repository.

| It lists | Which can become |
|---|---|
| Your corrections, with what the agent had just done | A code rule, or a command rule |
| Risky commands the agent ran: force pushes, skipped hooks, hard resets, recursive deletes, publishes | A command rule |
| How often each rule steered | A rule to tune, not a new one |
| Standards files (CLAUDE.md, AGENTS.md, CONTRIBUTING.md, linter configs, Cursor and Copilot instructions) | Rules from the standards they state |

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

</details>

## Two kinds of rule

Code rules check the files the agent edits; command rules check the commands it runs.

- **A code rule** is a `rerail.policy/v1` document: globs for scope, clauses that match text, paths or syntax.
- **A command rule** is a conversation rule that matches `input.command`.

<details>
<summary>What each kind catches</summary>

- **Code rules** catch cross-layer imports, a call that bypasses a helper, a blocking call in async code, an old path
  still written in a `Justfile`, or a file in the wrong folder. A clause is a `regex` over a file's text or path, a
  `file` query, or a syntax query over Python, JavaScript, TypeScript, PHP or Blade.
- **Command rules** catch force pushes, `--no-verify` or publishing.

</details>

Guides:

- [Writing a code rule](../skills/rerail-create/references/code-rules.md)
- [Writing path and text rules](path-and-text-rules.md): text and paths in any file
- [Architecture rule examples](architecture-rule-examples.md): rules that hold a design in place
- [Writing a command rule](../skills/rerail-create/references/conversation-rules.md)
- [tool-call-rules.md](tool-call-rules.md): regex technique for command rules

## Testing a draft

A draft is the rule plus the cases it must catch and pass; test it before adding it.

`rerail rules test` and `rerail rules add` read this envelope, not the bare rule. `rerail rules test --help` prints it.

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

<details>
<summary>Draft fields</summary>

| Field | What it holds |
|---|---|
| `kind` | `code` |
| `rule` | the rule document |
| `folder` | optional: the repository folder to add the rule at, `""` for the whole repository. Absent, a new rule goes to the whole repository and one already added stays where it is. |
| `cases.caught`, `cases.passed` | files the rule must catch, and must let pass: `{ "path", "content" }` |

</details>

[Writing path and text rules](path-and-text-rules.md#text-in-files) has complete drafts to start from.

```sh
rerail rules test draft.json            # validate, run the cases, list the matches across the repo; writes nothing
rerail rules test rule.rr               # a .rr rule file: run the tests it carries (.test ... .end)
rerail rules test draft.json --json     # the same, listing every match and every file the rule could not check
rerail rules add draft.json             # store it in your library and add it, on, to this repo
rerail rules approve <id>               # after reading it: it steers instead of running in Log
rerail rules export --output rules.json # your rules, to read or edit one
rerail rules show <id> > draft.json     # one rule as a draft, a By Rerail one too; adding it back stores your copy
```

<details>
<summary>What the report shows and skips</summary>

The text report shows the first five matches and the count; `--json` lists all. Path and `file` matches name the file
with no line. Binary files in scope are counted; no rule reads them.

The repo pass skips, and counts without naming:

- hidden files and folders;
- files whose names usually hold secrets (`*.pem`, `id_rsa*`, `.ssh/`, …);
- files Git ignores, unless the repository's Follow .gitignore setting is off.

[Reference](reference.md#scan-and-settings-controls) has the full list.

</details>

**Before adding a draft, check:**

- **Cases.** All pass, including a near miss: code that looks like a violation but is not.
- **Repo matches.** Each is a real violation, and there are a handful, not hundreds. Hundreds mean the repo's
  convention says otherwise.
- **Message.** Following it cannot break working code.

[Rule quality](../skills/rerail-create/references/rule-quality.md) has the gates, the five kinds of harmful steering, and
the fix ladder for a misfiring rule.

## Steer or Log

A rule's mode decides whether the agent hears about a violation.

- **Steer (`steer`)** tells the agent in the session. Use it when you are sure of the rule.
- **Log (`log`)** records it and tells no one. Use it on trial, or when existing code breaks it too often.

A rule from `/rerail-create` is never `block`. It belongs to the repository it was made in, unless you keep it to one
worktree.

## Approving a rule

A rule you add with `rerail rules add`, or an agent writes, runs in Log until a person approves its message.

Say yes to `/rerail-create` after its read-back, or approve on the rule's page in the dashboard. An edit needs approval
again.

<details>
<summary>Why, and what else waits for approval</summary>

Every coding agent in the repository acts on a steer, so a person reads it first. On your yes, `/rerail-create` runs
`rerail rules approve <id>`. The same approval applies to a worktree's own rules and to packs Rerail does not publish
([approval](rule-scoping.md#approval)).

</details>

## A rule for one worktree only

A JSON file in the worktree's `.rerail/rules/` folder is a rule that runs in that worktree only.

Use it for a branch experiment, a migration in progress, or a convention on trial. Each file holds one rule document
(the draft's `rule`, without the cases).

```text
feature/                  a linked worktree of the repository
└── .rerail/
    └── rules/
        ├── no-legacy-client.json
        └── use-new-logger.json
```

**Keep the folder out of Git** unless you mean to share it: add `.rerail/rules/` to `.git/info/exclude`.

<details>
<summary>How worktree rules run</summary>

- **Scope.** Each rule is on, in its own mode, for every folder in the worktree. The main checkout and other worktrees
  never see it.
- **Edits.** A changed file applies at the agent's next edit, with nothing else to do.
- **Setup.** Nothing to install or add, but the worktree must be started and its repository set up, as for any rule.
- **Approval.** Each rule runs in Log until you run `rerail rules approve <id>` in the worktree, and again after each edit.
- **Git.** A committed `.rerail/rules/` runs in every checkout of that branch with Rerail started, and its messages reach
  the coding agent there.

`/rerail-create` writes a rule here when you ask for this worktree only, or pick it under **Change something** in the
read-back.

</details>

<details>
<summary>Checking what Rerail loaded</summary>

`rerail rules test .rerail/rules/<name>.json` validates a hand-written file and counts its repo matches, as it does a
draft. It has no cases to run, so a draft with cases is still how you prove a rule.

`rerail packs list` in the worktree shows what Rerail loaded, whether it runs, and why a file was left out (invalid, an id
another rule already has, a rule the engine cannot run):

```text
This worktree's own rules, in /src/acme-web-feature/.rerail/rules (pack worktree, here only): 2
  Not running: this repo is not set up yet, so no rule runs here. `rerail start` sets it up.
  local:no-legacy-client (code, steer)
    waiting for approval, runs in Log until then: rerail rules approve local:no-legacy-client
  local:use-new-logger (code, log)
  left out: draft.json: clauses Too small: expected array to have >=1 items
```

The `Not running` line shows only while Rerail is off in the worktree or the repo is not set up.

In the dashboard for the worktree (`rerail manage`), these rules are the pack **This worktree**. Turning the pack or a
rule on or off there changes this worktree only. Its Overview lists each left-out file and why, until it is fixed or
removed.

</details>

[A worktree's own rules](rule-packs.md#a-worktrees-own-rules) has every check Rerail makes.
