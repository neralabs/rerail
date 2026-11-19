# The `rerail` pack in `.rerail`

The 34 rules of `packs/rerail/rules/*.json`, written for this repository's own conventions, translated by hand. Each
translation is checked against its JSON original by the translation suite (`docs/rule-syntax/packs/CONVENTIONS.md`,
"Checking against the original"); `equivalence.json` beside this file is what that check gates on.

## How the rules are written

- A `forbid.regex` clause over `file.content` is `CODE(m) :- source(_, sp), match_all("<pattern>", sp, m).`, the
  pattern and the message copied exactly from the JSON; an anchor on a named group (`capture.X`) reports the group's
  span (`match_group`). The finding sits where the match starts, as the linter's does.
- An `ast` clause is Datalog over the tree. A JSON `regex` on a node is ast-grep's search over the node's text
  (`match("(?s:.*)(?:…)(?s:.*)", n)`, or a glob where one says exactly the same); `inside` without `stopBy` is the
  parent, with `stopBy: end` any ancestor (`inside`); `has` without `stopBy` a child (of a field, where it names one).
- A policy runs over one language's files, so a rule whose clauses span languages is several policies that all
  declare the rule's `.policy`: `<rule-id>.rerail` and `<rule-id>.<language>.rerail`. A clause over files of no
  grammar is a text-only policy (`.scope text`, no `.language`).
- Each file carries its tests: the pack's caught and passing examples (`packs/rerail/pack.json`), a violating and a
  clean case per clause, and a file the scope leaves out.

Batch notes: [a](NOTES-rerail-a.md), [b](NOTES-rerail-b.md), [c](NOTES-rerail-c.md), [d](NOTES-rerail-d.md),
[e](NOTES-rerail-e.md). The first five rules are below.

## The first five

| Rule | Differs from the JSON | Validation |
|---|---|---|
| `rerail-fixtures-root` | No. The JSON's `inside` without `stopBy` is the direct parent (`child`), with `stopBy: end` any ancestor (`inside`). | tests pass; same on the examples and the repository (21 findings) |
| `rerail-start-state` | No: three regex clauses. | tests pass; same on the examples (1) and the repository (0) |
| `rerail-store-reads-in-documents` | No: one regex clause. | tests pass; same on the examples (1) and the repository (11) |
| `rerail-walk-limits` | No: two regex clauses. | tests pass; same on the examples (2) and the repository (2) |
| `rerail-walk-limits-native` | No: two regex clauses. | tests pass; same on the examples (1) and the repository (8) |

The repository corpus is this repository at 7efcac9c67 (the scopes' directories: `.github`, `conformance`,
`integration`, `native` (Rust files), `packs`, `scripts`, `src`, `test`, `ui`, and `package.json`).

`site/check_rr.py` learned that `match_all` binds its match (`match_all("re", sp, m)` binds `m`; over a symbol, the
ordinal and offsets): it reported every regex rule here as ungrounded, and three earlier files too.

## Not translated: what the engine lacks

| Rule | The gap | Smallest example | Kind of change |
|---|---|---|---|
| `rerail-workflow-script-facts`, `rerail-workflow-shared-conditions` | Every scope is under `.github/`, and the engine's corpus never lists a file under a directory whose name starts with `.`: `corpus_files` in `native/src/code_rule_dsl/logic/plan/corpus.rs` skips them (both the Git listing and the walk). A policy with `.scope include ".github/workflows/**/*.yml"` and `.scope text` the same reads 0 files; the same test with its fixture at `wf/a.yml` passes, at `.github/workflows/a.yml` it fails: `.expect PROBE at ".github/workflows/a.yml":2: no findings`. | `.github/workflows/a.yml` holding `uses: ./.github/actions/php`, a rule matching `uses:` | A design question for the engine's owner, not made here: three definitions disagree. The product's walk skips only `IGNORED_DIRECTORIES` (`src/engine/code-rule/ignored-directories.ts`: `.git`, `.hg`, `.svn`, `.rerail`, `.venv`, `.next`, `.turbo`, `__pycache__`, `node_modules`, `dist`, `build`, `coverage`, `target`, `vendor`); `lib/core.rerail` documents `.git`, `.venv`, `venv`, `__pycache__` and `tmp`; the engine skips every hidden directory and `target`. Which set the engine follows changes the files of every run. |
| `rerail-pack-data-through-publish` | Not a code rule: it matches a coding agent's tool call (`match` on `input.command`, `sample: { source: "tool:*", boundary: "tool-call" }`), and `.rerail` has no relation for a tool call's input. `rerail-engine translate-json` makes a policy with no rule, which `run-policies` refuses: `no .violation declarations`. | the Bash command `npm run fixtures:push -- --pack react --tool eslint-plugin-react` | A design question: tool-call events as an input of the rule language (a relation per event field) are not part of the engine's model. |
