# How Rerail checks your agent

What checks the agent's code, what a rule is under the hood, what happens when a fix breaks another rule, how Rerail
relates to your linters, and whether a rule can hold an architectural decision.

This page describes Rerail's default behaviour. Some of it changes with your coding agent and your settings: see
[What depends on your setup](#what-depends-on-your-setup).

## Is it a model, or deterministic checks?

Deterministic. A code rule is a pattern over a file's text, path or syntax tree, and Rerail's native engine matches
it. No model decides whether code breaks a rule: not your coding agent, and not a hosted LLM. The same edit against
the same rules gives the same findings. The engine's limits on work and file size count the input, not the time a check
takes, so a busy machine does not change the result.

Your coding agent has two main parts in it:

- **Fixing.** When a rule on Steer is broken, Rerail tells the agent the rule, where, and what is wrong. The agent
  makes the fix.
- **Writing rules.** `/rerail-create` has the agent draft a rule from your description. Rerail tests the draft
  against the repository, and a person approves what it says before it steers.

<details>
<summary>Where a model is involved</summary>

Conversation rules, which match what the agent says or does by meaning, score similarity with a small embedding model
bundled with Rerail, run on your machine. They are [internal testing](internal-testing.md): experimental, and off
unless you turn them on. Code rules never use it.

</details>

## What is a rule, under the hood?

A code rule is a `rerail.policy/v1` JSON document. A rule pack can also ship rules in Rerail's own `.rerail`
language. A rule says:

- **Where it applies:** globs for the files it checks.
- **What is wrong:** clauses such as "this must not appear", "each of these needs that", or "no more than this many",
  built from regex over a file's text or path and from queries over its syntax tree.
- **What to tell the agent:** a message for each finding, and for some kinds of clause a suggested fix.
- **Its mode:** Steer (tell the agent) or Log (only record it).

Rules are checked on your machine, by Rerail's engine running locally. It makes no network requests to check them.

<details>
<summary>What happens to an edit</summary>

1. **Capture.** Rerail's hooks or extension in your coding agent report each tool call. Before an edit, Rerail
   records the file as it was, so it can tell which lines the agent changed.
2. **Parse.** The engine parses the edited file into a syntax tree for the languages it supports. Regex clauses read
   any text file.
3. **Match.** Every rule that applies to the file runs over it. Compiled rules stay in memory between edits.
4. **Keep the agent's lines.** By default, only findings on lines the edit added or changed are kept. Code that
   broke a rule before the agent touched it is not reported.
5. **Steer or log.** A rule on Steer sends its findings to the agent. A rule on Log only records them, for the
   dashboard.

</details>

[Writing rules](writing-rules.md) covers drafting and testing one. The [code-rule DSL
reference](code-rule-dsl-reference.html) documents every construct.

## A steer leads to a fix, and the fix breaks another rule. What happens?

The fix is an edit like any other, so Rerail checks it against every rule on in that folder. If the fix breaks rule B,
and rule B is on Steer, the agent is steered on it and fixes again.

- **A steer is advice, not a block.** By default the edit stays on disk. The agent is asked to fix it, or to tell
  you in one line why it won't and record that reason with `rerail feedback`.
- **A finding goes out once.** A newer edit to a file drops a steer still waiting to be sent for it, so the finding
  arrives once if it is still there, and not at all if the newer edit fixed it. By default, a finding the agent has
  seen is not repeated by edits elsewhere in the file.
- **Fixed findings are not tracked.** Rerail keeps no "fixed" state. An unfixed finding is reported again when a
  later edit changes the lines it is on.

Rerail sets no limit on how many steers a chain of fixes can bring. What happens when the agent finishes its turn
depends on the agent: Rerail continues a Claude Code turn at most once, never continues a Codex turn, and continues an
OMP or Pi run whenever steers are waiting. Otherwise a steer waits for the agent's next step.
[In-session surfaces](in-session-surfaces.md#steer) has the details for each agent.

<details>
<summary>Rules that contradict each other</summary>

Rerail does not detect rules that contradict each other, at any point. If rule A requires what rule B forbids, the
agent can satisfy only one, and the other keeps steering. The agent may decline a steer, and a reason it records with
`rerail feedback` shows on the dashboard. Settle the conflict by turning one rule to
Log or off in the folders where they overlap ([Rule scoping](rule-scoping.md#rules-attachments-and-settings)).

</details>

## Does Rerail run my linters?

No. Rerail does not run ESLint, Ruff, Clippy or any other linter. Its rule packs port linters' rules into rules for
its own engine: the `javascript` pack ports rules from ESLint, for example, and `python` from Ruff.

- **Ported rules are checked against the linter.** Before a ported rule ships, it is compared with the linter at
  the version the pack names, and every difference is declared with evidence
  ([Linter-equivalent](rule-packs.md#linter-equivalent)).
- **Your lint config is read, not run.** If you let set-up read the repository, it reads your linters' configs as
  data and turns pack rules on and off to match, where it can read them. A setting you made yourself is never
  changed. `rerail packs calibrate` reads the configs again later ([The repository's own lint
  config](rule-scoping.md#the-repositorys-own-lint-config)).
- **Suppression comments work.** By default a ported rule respects its linter's comments (`eslint-disable`, `noqa`,
  ...). You can set a rule or pack to ignore them.

Because the rules run in Rerail's engine, they check each edit as the agent makes it, with no linter to install.

## Can a rule hold an architectural decision?

Yes, when the code shows the decision. Rules about layers, dependency direction, and calls that must go through a
service or repository are common ones. [Architecture rule examples](architecture-rule-examples.md) has tested rules for
design decisions that maintainers of well-known open-source projects enforce in review. `/rerail-create` can write
them from your design docs and review comments, following its [architecture rules
guide](../skills/rerail-create/references/architecture-rules.md).

Most of what a rule reads is in one file: its syntax tree, path and text. A design decision is easiest to check when
that file shows it:

1. **In the same file:** a call sits inside the class, function or block it must not be in.
2. **In the names:** a module or package marks the boundary, which a rule reads through the file's imports, under
   any alias.
3. **In the layout:** a folder or file name says which layer a file is in.

Some constructs look further: at the nearest package manifest, or at how functions call each other across the
repository, in the languages that support them. A rule does not know a value's type the way a type checker does. For
what a rule cannot see, keep the check where it is today, in review or a test ([What a rule can and can't
check](architecture-rule-examples.md#what-a-rule-can-and-cant-check)).

## What depends on your setup

The answers above are Rerail's defaults. These change them:

- **Your coding agent.** Where a steer appears, and whether Rerail continues a turn so the agent can act on it,
  differ between Claude Code, Codex, Cursor, OMP and Pi. In Codex, an edit made with `apply_patch` is checked when
  the turn ends, not right after the edit.
- **Rule settings.** Each rule or pack can be on or off, on Steer or Log, per repository, folder or checkout. A rule
  nobody has approved yet (one you or an agent wrote, or a third party's) runs in Log, so it never steers.
- **A checkout's audit settings.** `rerail settings` can report findings on the whole edited file instead of only the
  changed lines, or put the checkout on Log, so nothing steers.
- **Your lint config.** Which ported rules start on depends on which linters your repository configures, and whether
  Rerail can read each config.
- **Experimental features**, off unless you turn them on ([Internal testing](internal-testing.md)): conversation
  rules, Block mode (it steers for now), relational rules across the repository, and steer strategies, one of which
  undoes an edit that breaks a rule instead of leaving it on disk.
