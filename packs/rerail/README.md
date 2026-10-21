# Rerail rule pack

Rules for a repository that holds Rerail rule packs, as this one does under `packs/`. They steer the coding agent while
it writes a rule's violation message, the text an agent reads when that rule steers. Set-up offers the pack to a
repository with a `packs/*/pack.json`; it is a `repository` pack, added for the whole repository, and once a repository
adds it its rules are on (see `docs/rule-packs.md`). Every document has `mode: "steer"`.

| Rule | Flags a message that |
|---|---|
| `rerail-message-tool-names` | names an upstream tool |
| `rerail-message-rerail-name` | names Rerail |
| `rerail-message-shouting` | writes NEVER, MUST or ALWAYS in capitals |

Each problem is its own rule, so each can be turned off or set to Log on its own, per repository or folder, and a new
one is one more document. The rest of the [message criteria](../../docs/pack-format.md#message) are left to review.

All three read every violation message in `packs/*/rules/*.json` and `packs/*/candidates/rules/*.json` (not the probes
under `candidates/probes/`), each case of one an option chooses included but not the option's name, and report a
violation once, at its first offending word. A word in backticks or single quotes is code or a configured value, and
passes; double quotes don't count. The rest of the document (its `id`, `code`, `suppress`, patterns and options) is
never read.

`test/engine/code-rule/rerail-message-rules.test.ts` runs each rule on the cases it must catch and the near misses it
must let through.

## `rerail-message-tool-names`

Flags a message that names an upstream tool: eslint, oxlint, biome, ruff, bandit, pylint, flake8, clippy, sqlfluff,
phpstan, psalm, phpcs, tlint, bladestan, lizard, blade-parser or unicorn, in any case and as a whole word (so also
`typescript-eslint`). A pasted rule id is flagged when it starts with a tool's name. The steer says to reword the
message without the tool or a rule id, and to backtick only code the reader must write, such as a suppression comment.

What it gets wrong:

- **A message holding its own rule id is flagged only when the id names a tool**: 523 of the 595 ids in this
  repository's packs do; the `n-`, `next-`, `react-`, `vitest-` and `complexity-` ones don't. A rule reads JSON as
  text only, and a regex cannot compare the document's `id` with its message.
- **Backticks hide any word**, so backticking a tool name or a rule id silences it without fixing the message.
- The [quoting limits](#what-all-three-get-wrong) below.

## `rerail-message-rerail-name`

Flags a message that names Rerail, in any case: the reader already sees the name with every finding. A
`` `// rerail-ignore` `` comment in backticks passes.

What it gets wrong: backticks hide the name as they do a tool's, and the [quoting limits](#what-all-three-get-wrong)
below.

## `rerail-message-shouting`

Flags NEVER, MUST or ALWAYS in capitals before a lower-case word (`MUST NOT turn` included). A SQL keyword such as
`GENERATED ALWAYS AS` passes. The steer says to write the word in lower case.

What it gets wrong:

- **Shouting before anything but a lower-case word passes**: `NEVER.` at the end of a sentence, `NEVER, ever`.
- The [quoting limits](#what-all-three-get-wrong) below.

## What all three get wrong

- **An apostrophe that pairs with no quote**, such as a Rust lifetime (`'a`) or `print()'s`, ends what a rule reads of
  a message, unless a quote later in the message pairs with it: then what that quote opened counts as unquoted.

## How the pattern reads a message

There is no JSON language: each rule is one regex over the file's text, and the three share everything but the words
they look for. The pattern finds a `violation`'s `message` key (in the violation itself or in one an option chooses),
then its string, or a string value under a key that doesn't start with `$` inside the object an option chooses,
skipping whole objects up to seven deep. It reads that string as tokens: words (with apostrophes inside them),
`$metavariables` and `${…}`, JSON escapes, backtick spans, and single-quoted spans that open and close next to a
non-word character. A word counts only outside every span. To add a tool, add it to the word list at the end of
`rerail-message-tool-names`' pattern; the rest is shared and changes in all three at once.
