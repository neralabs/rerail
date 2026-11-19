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

## The 31 translated rules

Each rule's tests pass (`rerail-engine test-rr`, 125 tests), and its findings are the same set as its JSON original's,
every rule run together on each side, on three corpora: the pack's caught examples, its passing examples (no finding
on either side for any rule), and this repository at 7efcac9c67 (the scopes' directories: `.github`, `conformance`,
`integration`, `native` (Rust files), `packs`, `scripts`, `src`, `test`, `ui`, and `package.json`). The counts are
findings both sides report (keys: rule, path, line, code); no rule has a finding on one side only. Files: the rule's
policies, `.rerail` for `<rule-id>.rerail` and the language part for `<rule-id>.<part>.rerail`.

| Rule | Files | Tests | Caught example | Repository | Notes |
|---|---|---|---|---|---|
| `rerail-command-sigil` | `.rerail` | 4 | same (1) | same (0) | [b](NOTES-rerail-b.md) |
| `rerail-conformance-boundary` | `.rerail`, `text` | 6 | same (1) | same (0) | [b](NOTES-rerail-b.md) |
| `rerail-conformance-pinned-data` | `.rerail` | 3 | same (1) | same (0) | [b](NOTES-rerail-b.md) |
| `rerail-fixed-value-lists` | `.rerail` | 4 | same (1) | same (0) | [a](NOTES-rerail-a.md) |
| `rerail-fixtures-root` | `.rerail` | 3 | same (1) | same (21) | below |
| `rerail-generator-placement` | `php`, `python`, `.rerail`, `shell` | 10 | same (1) | same (0) | [e](NOTES-rerail-e.md) |
| `rerail-git-no-lazy-fetch` | `.rerail` | 6 | same (1) | same (0) | [c](NOTES-rerail-c.md) |
| `rerail-grammar-node-kinds` | `.rerail` | 4 | same (1) | same (89) | [d](NOTES-rerail-d.md) |
| `rerail-home-file-names` | `.rerail` | 3 | same (1) | same (0) | [a](NOTES-rerail-a.md) |
| `rerail-home-file-paths` | `.rerail` | 3 | same (1) | same (0) | [a](NOTES-rerail-a.md) |
| `rerail-host-sdk-imports` | `.rerail` | 4 | same (3) | same (0) | [a](NOTES-rerail-a.md) |
| `rerail-language-branching` | `.rerail`, `typescript` | 5 | same (1) | same (109) | [d](NOTES-rerail-d.md) |
| `rerail-language-facts-from-registry` | `.rerail`, `typescript` | 5 | same (1) | same (17) | [d](NOTES-rerail-d.md) |
| `rerail-no-code-execution` | `.rerail` | 4 | same (2) | same (0) | [b](NOTES-rerail-b.md) |
| `rerail-no-message-classification` | `.rerail`, `rust` | 6 | same (1) | same (11) | [e](NOTES-rerail-e.md) |
| `rerail-origin-ranges` | `.rerail` | 2 | same (1) | same (2) | [d](NOTES-rerail-d.md) |
| `rerail-pack-data-helper-copies` | `.rerail` | 3 | same (1) | same (0) | [c](NOTES-rerail-c.md) |
| `rerail-pack-data-runtime-names` | `.rerail` | 3 | same (1) | same (0) | [c](NOTES-rerail-c.md) |
| `rerail-pack-package-names` | `.rerail` | 3 | same (1) | same (0) | [a](NOTES-rerail-a.md) |
| `rerail-pack-rule-test-placement` | `.rerail` | 3 | same (1) | same (0) | [a](NOTES-rerail-a.md) |
| `rerail-provenance-writers` | `.rerail`, `typescript` | 6 | same (1) | same (1) | [c](NOTES-rerail-c.md) |
| `rerail-rule-documents-in-packs` | `.rerail` | 3 | same (1) | same (537) | [c](NOTES-rerail-c.md) |
| `rerail-session-paths` | `.rerail` | 4 | same (1) | same (2) | [b](NOTES-rerail-b.md) |
| `rerail-start-state` | `.rerail` | 3 | same (1) | same (0) | below |
| `rerail-store-reads-in-documents` | `.rerail` | 2 | same (1) | same (11) | below |
| `rerail-table-probes-in-storage` | `.rerail` | 3 | same (1) | same (0) | [a](NOTES-rerail-a.md) |
| `rerail-ui-api-precedence` | `.rerail` | 3 | same (2) | same (7) | [e](NOTES-rerail-e.md) |
| `rerail-ui-color-tokens` | `html`, `javascript`, `.rerail` | 7 | same (2) | same (0) | [e](NOTES-rerail-e.md) |
| `rerail-ui-pure-modules` | `.rerail` | 4 | same (2) | same (0) | [e](NOTES-rerail-e.md) |
| `rerail-walk-limits` | `.rerail` | 3 | same (2) | same (2) | below |
| `rerail-walk-limits-native` | `.rerail` | 3 | same (1) | same (8) | below |

Where the repository has no finding, the batches also ran each test's fixtures through the original: all agree. Two
rules follow the JSON where it reports more than its intent (both sides report; the tests pin it):
`rerail-language-branching` reports `EMBEDDING_BRANCH` beside `LANGUAGE_BRANCH` on a dialect comparison outside
`embed/` ([d](NOTES-rerail-d.md)), and `rerail-ui-pure-modules` reports `window` read from a destructured parameter
([e](NOTES-rerail-e.md)). Per-rule detail: the batch notes [a](NOTES-rerail-a.md), [b](NOTES-rerail-b.md),
[c](NOTES-rerail-c.md), [d](NOTES-rerail-d.md), [e](NOTES-rerail-e.md); the first five rules are below. New
libraries: `lib/tsx/rerail-module-loads.rerail` (b), `lib/rust/rerail-native.rerail` (d), `lib/rr/rerail-ui-colours.rerail` (e).

### The first five

| Rule | Differs from the JSON |
|---|---|
| `rerail-fixtures-root` | No. The JSON's `inside` without `stopBy` is the direct parent (`child`), with `stopBy: end` any ancestor (`inside`). |
| `rerail-start-state` | No: three regex clauses. |
| `rerail-store-reads-in-documents` | No: one regex clause. |
| `rerail-walk-limits` | No: two regex clauses. |
| `rerail-walk-limits-native` | No: two regex clauses. |

`site/check_rr.py` learned that `match_all` binds its match (`match_all("re", sp, m)` binds `m`; over a symbol, the
ordinal and offsets): it reported every regex rule here as ungrounded, and three earlier files too.

### Engine faults met on the way (worked around, not changed)

- `run-policies` panics with `no declaration of within` (`native/src/code_rule_dsl/logic/plan/fields.rs`, `width`)
  when `rerail-ui-pure-modules` read `within(n, a)` and ran together with another JavaScript policy
  (`rerail-ui-api-precedence` or `rerail-ui-color-tokens.javascript`); each ran alone. The rule now reads the binders
  through `inside` with their kinds.
- `test-rr` over the whole directory fails tests with `unable to open database file` under the default limit of
  1,024 open files; with `ulimit -n 65536` all 125 pass.

## Not translated: what the engine lacks

| Rule | The gap | Smallest example | Kind of change |
|---|---|---|---|
| `rerail-workflow-script-facts`, `rerail-workflow-shared-conditions` | Every scope is under `.github/`, and the engine's corpus never lists a file under a directory whose name starts with `.`: `corpus_files` in `native/src/code_rule_dsl/logic/plan/corpus.rs` skips them (both the Git listing and the walk). A policy with `.scope include ".github/workflows/**/*.yml"` and `.scope text` the same reads 0 files; the same test with its fixture at `wf/a.yml` passes, at `.github/workflows/a.yml` it fails: `.expect PROBE at ".github/workflows/a.yml":2: no findings`. | `.github/workflows/a.yml` holding `uses: ./.github/actions/php`, a rule matching `uses:` | A design question for the engine's owner, not made here: three definitions disagree. The product's walk skips only `IGNORED_DIRECTORIES` (`src/engine/code-rule/ignored-directories.ts`: `.git`, `.hg`, `.svn`, `.rerail`, `.venv`, `.next`, `.turbo`, `__pycache__`, `node_modules`, `dist`, `build`, `coverage`, `target`, `vendor`); `lib/core.rerail` documents `.git`, `.venv`, `venv`, `__pycache__` and `tmp`; the engine skips every hidden directory and `target`. Which set the engine follows changes the files of every run. |
| `rerail-pack-data-through-publish` | Not a code rule: it matches a coding agent's tool call (`match` on `input.command`, `sample: { source: "tool:*", boundary: "tool-call" }`), and `.rerail` has no relation for a tool call's input. `rerail-engine translate-json` makes a policy with no rule, which `run-policies` refuses: `no .violation declarations`. | the Bash command `npm run fixtures:push -- --pack react --tool eslint-plugin-react` | A design question: tool-call events as an input of the rule language (a relation per event field) are not part of the engine's model. |
