# Guardrails rule pack

Rules on what a coding agent does to the checks that guard a repository. One rule today:
`guardrails-no-new-lint-suppression`, a conversation rule on the agent's tool calls. Set-up recommends the pack for
every repository (its match is any file, its category `repository`, so it is added for the whole repository); drop it
in the review, or pick packs yourself, to go without it.

Rerail's ported rules honour the linters' own suppression comments (`suppressions: honour` per repository), so an
agent could silence a rule by writing one. This rule steers the agent, every time, when a file-writing tool call adds
such a comment: fix the code the linter flags instead; if the suppression is genuinely needed, leave it out, tell the
person which rule and why, and let them add it themselves. It steers and never blocks.

## What counts as a suppression comment

| Linter | Comment |
|---|---|
| ESLint | `eslint-disable`, `eslint-disable-line`, `eslint-disable-next-line` (any comment form), `/* eslint <rule>: off */` and `: 0` |
| Oxlint | `oxlint-disable` and its variants |
| Biome | `biome-ignore` and its variants |
| TypeScript | `@ts-ignore`, `@ts-expect-error`, `@ts-nocheck` |
| Ruff, Flake8 | `# noqa` (any case) |
| Bandit | `# nosec` |
| mypy, Pyright | `# type: ignore` |
| Pylint | `# pylint: disable` and `disable-next` |
| PHPStan | `@phpstan-ignore`, `@phpstan-ignore-line`, `@phpstan-ignore-next-line` |
| Psalm | `@psalm-suppress` |
| PHP_CodeSniffer | `phpcs:ignore`, `phpcs:disable` |

## Which tool calls, and which text

Only tool calls whose name is a file write or edit, case-insensitively `write`, `edit`, `multiedit`, `ast_edit` or
`apply_patch` (the names Rerail's code rules treat as edits), so a `grep eslint-disable` in a shell never fires. The
rule reads the text the call adds, where the arguments say which text that is:

- `content` (Claude Code, Cursor and OMP `Write`): the whole file written.
- `new_string` (Claude Code `Edit`): the replacement only, so an edit that removes a suppression does not fire.
- each `new_string` of `edits` (Claude Code `MultiEdit`): matched in the call's JSON arguments, from a
  `"new_string":"` key to the end of that string value.
- the `+` lines of a patch in `command`, `patch` or `input` (Codex `apply_patch`, OMP `edit`): added lines only;
  removed (`-`) and context lines do not fire.

## Limits

- A `Write` replaces a whole file, and an `Edit`'s `new_string` often repeats lines around the change: a suppression
  that was already there and is carried through fires too. The rule cannot compare the text before and after.
- Text that mentions a suppression comment, such as documentation about `# noqa`, fires the same as a real one.
- `MultiEdit` is matched on its JSON arguments, where a tab is `\t`: `#<tab>noqa` there is missed.
- `/* eslint <rule>: off */` is matched for one unquoted rule set to `off` or `0`; `"off"`, `["off"]` and lists of
  rules are not.
- A tool whose arguments use other names than these, or a shell command that writes a file (`echo ... >> a.py`), is
  not read.
