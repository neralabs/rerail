# Rerail rule pack

Rules for a repository that holds Rerail rule packs, as this one does under `packs/`. They steer the coding agent while
it writes a rule's violation message, the text an agent reads when that rule steers. Set-up offers the pack to a
repository with a `packs/*/pack.json`; it is a `repository` pack, added for the whole repository, and once a repository
adds it its rules are on (see `docs/rule-packs.md`). Every document has `mode: "steer"`.

The message criteria (`docs/pack-format.md`, "Message") are for review, and most of them take judgement. These rules
check the part of criterion 5 that takes none. That criterion asks a message to leave out what the block around it
already shows (the rule id, the location, "Rerail", "fix it or explain") and to name no tool, rule code, tool version or
matcher term outside quoted code, because a message names the change and the reader acts on the code, not on the tool
behind the rule.

| Rule | Flags a message that |
|---|---|
| `rerail-message-tool-names` | names an upstream tool |
| `rerail-message-rerail-name` | names Rerail |

Each is its own rule, so each can be turned off or set to Log on its own, per repository or folder, and a new one is
one more document.

Both read every violation message in `packs/*/rules/*.json` and `packs/*/candidates/rules/*.json` (not the probes under
`candidates/probes/`), each case of one an option chooses included but not the option's name, and report a violation
once, at its first offending word. A word in backticks or single quotes is code or a configured value, and passes;
double quotes don't count. The rest of the document (its `id`, `code`, `suppress`, patterns and options) is never read.

`test/engine/code-rule/rerail-message-rules.test.ts` runs each rule on the cases it must catch and the near misses it
must let through.

## `rerail-message-tool-names`

Flags a message that names an upstream tool: eslint, oxlint, biome, ruff, bandit, pylint, flake8, clippy, sqlfluff,
phpstan, psalm, phpcs, tlint, bladestan, lizard, blade-parser or unicorn, in any case and as a whole word (so also
`typescript-eslint`). That catches the old bare messages, such as `Bandit B113 (request_without_timeout)`, and a pasted
rule id that starts with a tool's name. The steer says to name the change instead of the tool or its rules, and to
backtick a tool name only in code the reader must write, such as a suppression comment.

What it gets wrong:

- **A rule id or code is flagged only beside a tool name.** 523 of the 595 ids in this repository's packs contain one;
  the `n-`, `next-`, `react-`, `vitest-` and `complexity-` ones don't, and a bare code such as `B113` doesn't. A rule
  reads JSON as text only, and a regex cannot compare the document's `id` with its message.
- **Backticks hide any word**, so backticking a tool name or a rule id silences it without fixing the message.
- The [quoting limits](#what-both-get-wrong) below.

## `rerail-message-rerail-name`

Flags a message that names Rerail, in any case: the block around every message already shows where the finding came
from. A `` `// rerail-ignore` `` comment in backticks passes.

What it gets wrong: backticks hide the name as they do a tool's, and the [quoting limits](#what-both-get-wrong) below.

## What both get wrong

- **An apostrophe that pairs with no quote**, such as a Rust lifetime (`'a`) or `print()'s`, ends what a rule reads of
  a message, unless a quote later in the message pairs with it: then what that quote opened counts as unquoted.

## What the pack does not check

The rest of criterion 5 (a location, "fix it or explain", a tool version, a matcher term) and the other criteria. A
regex can't tell `line 42` or `1.24.0` in a message from text the change needs, and criterion 3, a risk stated only as
far as the rule can confirm it, is about what the rule can confirm: capitals such as NEVER are a symptom of overclaiming,
and writing them in lower case would not fix the claim. Those are for review.

## How the pattern reads a message

There is no JSON language: each rule is one regex over the file's text, and the two share everything but the words
they look for. The pattern finds a `violation`'s `message` key (in the violation itself or in one an option chooses),
then its string, or a string value under a key that doesn't start with `$` inside the object an option chooses,
skipping whole objects up to seven deep. It reads that string as tokens: words (with apostrophes inside them),
`$metavariables` and `${…}`, JSON escapes, backtick spans, and single-quoted spans that open and close next to a
non-word character. A word counts only outside every span. To add a tool, add it to the word list at the end of
`rerail-message-tool-names`' pattern; the rest is shared and changes in both at once.
