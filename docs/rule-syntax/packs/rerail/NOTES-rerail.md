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
- A tool-call rule (`sample.boundary: "tool-call"`) is a `.scope tool_calls` policy over the call's arguments
  (`tool_input(c, "command", s)`); each of the JSON's regexes, a search, is `match("(?s:.*)(?:R)(?s:.*)", s)`. The
  finding is at the call, its code the rule's id, as `translate-json` gives it; its tests feed calls (`.tool_call`).

## The 34 translated rules

Each rule's tests pass (`rerail-engine test-rr`, 162 tests), and its findings are the same set as its JSON original's,
every rule run together on each side by the translation suite (`npm run translations:check -- --pack rerail`), on
three corpora: the pack's caught and passing examples (no finding on either side for a passing one), and this
repository at 7efcac9c67 (the scopes' directories: `.github`, `conformance`, `integration`, `native/crates`,
`native/src`, `packs`, `scripts`, `src`, `test`, `ui`, `package.json` and `tsconfig.json`); the tool-call rule on its
examples as tool calls instead. The counts are findings both sides report (keys: rule, path, line, code; rule and call
for the tool-call rule); no rule has a finding on one side only. Files: the rule's policies, `.rerail` for
`<rule-id>.rerail` and the language part for `<rule-id>.<part>.rerail`.

| Rule | Files | Tests | Caught example | Repository | Notes |
|---|---|---|---|---|---|
| `rerail-command-sigil` | `.rerail` | 4 | same (1) | same (0) | [b](NOTES-rerail-b.md) |
| `rerail-conformance-boundary` | `.rerail`, `text` | 6 | same (1) | same (0) | [b](NOTES-rerail-b.md) |
| `rerail-conformance-pinned-data` | `.rerail` | 3 | same (1) | same (0) | [b](NOTES-rerail-b.md) |
| `rerail-fixed-value-lists` | `.rerail` | 4 | same (1) | same (0) | [a](NOTES-rerail-a.md) |
| `rerail-fixtures-root` | `.rerail` | 3 | same (1) | same (20) | below |
| `rerail-generator-placement` | `php`, `python`, `.rerail`, `shell` | 10 | same (1) | same (4) | [e](NOTES-rerail-e.md) |
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
| `rerail-pack-data-through-publish` | `.rerail` | 23 | same (1) | tool calls only | below |
| `rerail-pack-package-names` | `.rerail` | 3 | same (1) | same (0) | [a](NOTES-rerail-a.md) |
| `rerail-pack-rule-test-placement` | `.rerail` | 3 | same (1) | same (0) | [a](NOTES-rerail-a.md) |
| `rerail-provenance-writers` | `.rerail`, `typescript` | 6 | same (1) | same (1) | [c](NOTES-rerail-c.md) |
| `rerail-rule-documents-in-packs` | `.rerail` | 3 | same (1) | same (0) | [c](NOTES-rerail-c.md) |
| `rerail-session-paths` | `.rerail` | 4 | same (1) | same (2) | [b](NOTES-rerail-b.md) |
| `rerail-start-state` | `.rerail` | 3 | same (1) | same (0) | below |
| `rerail-store-reads-in-documents` | `.rerail` | 2 | same (1) | same (11) | below |
| `rerail-table-probes-in-storage` | `.rerail` | 3 | same (1) | same (0) | [a](NOTES-rerail-a.md) |
| `rerail-ui-api-precedence` | `.rerail` | 3 | same (2) | same (7) | [e](NOTES-rerail-e.md) |
| `rerail-ui-color-tokens` | `html`, `javascript`, `.rerail` | 7 | same (2) | same (0) | [e](NOTES-rerail-e.md) |
| `rerail-ui-pure-modules` | `.rerail` | 4 | same (2) | same (0) | [e](NOTES-rerail-e.md) |
| `rerail-walk-limits` | `.rerail` | 3 | same (2) | same (2) | below |
| `rerail-walk-limits-native` | `.rerail` | 3 | same (1) | same (8) | below |
| `rerail-workflow-script-facts` | `.rerail` | 9 | same (1) | same (0) | below |
| `rerail-workflow-shared-conditions` | `.rerail` | 5 | same (1) | same (0) | below |

Where the repository has no finding, the batches also ran each test's fixtures through the original: all agree. Two
rules follow the JSON where it reports more than its intent (both sides report; the tests pin it):
`rerail-language-branching` reports `EMBEDDING_BRANCH` beside `LANGUAGE_BRANCH` on a dialect comparison outside
`embed/` ([d](NOTES-rerail-d.md)), and `rerail-ui-pure-modules` reports `window` read from a destructured parameter
([e](NOTES-rerail-e.md)). Per-rule detail: the batch notes [a](NOTES-rerail-a.md), [b](NOTES-rerail-b.md),
[c](NOTES-rerail-c.md), [d](NOTES-rerail-d.md), [e](NOTES-rerail-e.md); the first five rules and the last three are
below. New libraries: `lib/tsx/rerail-module-loads.rerail` (b), `lib/rust/rerail-native.rerail` (d),
`lib/rr/rerail-ui-colours.rerail` (e).

### The first five

| Rule | Differs from the JSON |
|---|---|
| `rerail-fixtures-root` | No. The JSON's `inside` without `stopBy` is the direct parent (`child`), with `stopBy: end` any ancestor (`inside`). Its scope follows the JSON's revision 2 (#872), which leaves out `native/src/code_rule_dsl/fixtures.rs`, where the two macros read the variable: the suite reported the macros' file as translation-only until the translation followed. |
| `rerail-start-state` | No: three regex clauses. |
| `rerail-store-reads-in-documents` | No: one regex clause. |
| `rerail-walk-limits` | No: two regex clauses. |
| `rerail-walk-limits-native` | No: two regex clauses. |

`site/souffle.py`, the grounding check `site/check_rr.py` runs, learned that `match_all` binds its match
(`match_all("re", sp, m)` binds `m`; over a symbol, `match_all("re", s, ordinal, start, end)` the ordinal and
offsets; no other arity, as the engine's `rr/check.rs`): it reported every regex rule here as ungrounded, and three
earlier files too.

### The last three

They waited on two engine changes: a run lists the files under `.github/` (it skips only the host's ignored
directories), and a policy reads a coding agent's tool calls (`.scope tool_calls`, `docs/rule-syntax/tool-calls.md`).

| Rule | Differs from the JSON |
|---|---|
| `rerail-workflow-script-facts` | No: five regex clauses over workflow and action YAML, read as text. |
| `rerail-workflow-shared-conditions` | No. The JSON's eleven clauses `release-skip-2` … `release-skip-12` each report the n-th line holding the release pull request skip; here one rule reports each line's first test when 1 to 11 earlier lines hold one, so a 13th repeat is not reported, as in the JSON (a test pins it). |
| `rerail-pack-data-through-publish` | No: the JSON's five alternatives and two exemptions, its dry-run exemption said once over the two script forms it guards. Both sides agree on its two examples and on its tests' 33 calls, each alternative and exemption: the suite runs every one of them through both on each check (the tool-calls corpus). Not checked: a command with a non-ASCII letter beside a `\b` (`match` reads `\b` as ASCII, `lib/core.rerail`; the product's regex as Unicode). |

### Engine faults met on the way (fixed in the engine, no workaround left)

Both fixes are their own pull requests, #861 and #863, merged into this branch to drop the workarounds; once they
are on main, this pull request's diff no longer holds them.

- A run of two or more policies lost the declaration of `within`, which `core.rerail` declares, and panicked with
  `no declaration of within`. Fixed in the lowering (`logic/rr/lower.rs`, `link_seeded`); `rerail-ui-pure-modules`
  reads `within(n, a)` as first written.
- A store held one open file per segment it had read, so `test-rr` over this directory reached the default limit of
  1,024 open files on a store kept between runs. Fixed in `rerail_blocks`: a mapped segment's file is closed once it
  is mapped. The 162 tests pass at the default limit.
