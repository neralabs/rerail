# Main's Python rules, translated for the engine

## Intro

This directory holds main's 537 Python JSON rules (`rerail.policy/v1`), translated mechanically into the
engine's `.rr` rules. It exists to test the Datalog engine against main on every rule, not only on the ones that
translate easily. Because a program does the translation, nobody picks or tunes a rule to suit the engine.

Each `<id>.rr` file is generated. Do not edit one by hand: the next regeneration overwrites it. A rule that the
translator cannot handle gets a hand-written file in `overrides/<id>.rr`, and that file replaces the generated
one. Each override starts with a comment that says why it exists.

Every rule keeps main's logic and regexes. Every rule says `.guests off`, because main matches only files and
never the strings inside them. No rule includes a library beyond the core relations the translator emits.

## How to regenerate

```sh
PACK_RULES=<main's pack>/rules ENGINE=<rerail-engine> RERAIL_RR_LIB=<engine lib dir> \
    docs/rule-syntax/site/regen_main_pack.sh [ids file]
```

The script does three things:

1. It runs `docs/rule-syntax/site/translate_rules.py` over the JSON files. That is `translate.py` (the display
   translation) followed by `engine_dialect.py` (the engine's dialect).
2. It copies the overrides over the generated files.
3. It reader-checks every file with `rerail-engine check-rr`.

The verdicts go to `reader-check.txt`, one line per rule: its status and the first line of any error. Old
generated files are moved to a directory under `/tmp`, not deleted.

## How the tape changes the translation

A tape stores a file's named nodes. It leaves out the tokens its grammar implies, such as the `,` between
arguments, the `as` in an import alias, or the `return` keyword. Main's rules often test those tokens through an
anonymous child or sibling: `has: {regex: "^,$"}` or `follows: {regex: "^as$"}`. The translator answers such a
test from three places:

- a named node;
- a token the tape stores, read with `op` or the parent's `token` list (the relation says `.stored`, with a
  reason);
- a token the tape leaves out, read from the source between stored children (`structural_before` and
  `structural_after`), tested as whole tokens.

A `layout` gap to the neighbouring token reads the line of the code next to the node from those same gaps. It
climbs to the parent where the gap reaches the parent's edge, and steps past comments unless the rule says
`comments: true`.

## Overrides

| Rule | Why |
|---|---|
| `bandit-b110-try-except-pass`, `bandit-b112-try-except-continue` | `pattern: {context, selector}` has no `.rr` form. The bare `except:` it selects is written as an except clause with no child but its block. |
| `ruff-ret505` … `ruff-ret508` | Main matches the `else`/`elif` keyword token and captures it. The tape leaves the keyword out, so the finding is at the clause (same line), and BRANCH is read from the source before its first child. |
| `ruff-com818-trailing-comma-on-bare-tuple` | A capture regex over a node of unknown kind: the statement kinds are named. The pattern_list trailing comma is read from the gap or the stored token. |
| `ruff-pyi010-non-empty-stub-body` | A capture regex over a node of unknown kind: the statement kinds are named. |
| `ruff-n806-non-lowercase-variable-in-function` | Main passes `NAME` in from the caller, and Datalog cannot, so the reader refuses it as ungrounded. Here `NAME` ranges over the nodes of the model's file, and the caller's join keeps its own. A stray `unset(MODEL)` is dropped. |

## Rules that still fail

Checked on 2026-11-13: 537 translated, 523 pass the reader (`reader-check.txt`), 507 were run on
50 files, and 502 of those run clean.

**The reader refuses 14 rules.**

- `ruff-f501`, `f502`, `f503`, `f506`, `f508`, `f509`, `f521`, `f523`, `f524`, `f525`, `ruff-fast003`, `ruff-up031`:
  main parses format strings with the `percentFormat` and `braceFormat` grammars. The engine has neither.
- `ruff-up030`, `ruff-up032`: they use the `formatArgs` term outside `ast()`, which has no `.rr` form.

**The engine refuses 13 rules at run time, on any input.**

- A JSON term in `ast()` does not lower: `formatArgs` (`ruff-f504`, `f505`, `f507`, `f522`), `bindings`
  (`ruff-up008`, `ruff-sim113`, `bandit-b615`), `redefines` (`ruff-f402`) and `resolves.beforeLocal` (`ruff-f823`).
- Not stratifiable, because a negation or an aggregate sits inside a recursive component: `ruff-ruf018`,
  `ruff-fbt001`, `ruff-n805`, `ruff-pyi051`.

**Rules that pass the reader but are too heavy, or abort, at scale.** Each run had caps of 2 minutes and 2 GB.

- `ruff-up028`: 1.7 GB on a single file. It was set aside after the 1-file run.
- `ruff-pie810` (over 2 GB) and `ruff-ann401` (2.5 GB): set aside after the 10-file run.
- `bandit-b606`, `bandit-b323`, `bandit-b304`: strict mode aborts on 50 files with
  `$rr_sd0 >= 1073741824 has no native step`.
- `ruff-c416`: runs past 2 minutes on 50 files when run alone.
- `ruff-air312`: aborts on 50 files when run alone, because the writer's queue is full (the memo's
  entries, about 200 MB).

On 50 files, two groups of about 30 rules went past 2 GB together, but each half passed. That is the
combined memory of many rules in one program, not one heavy rule.
