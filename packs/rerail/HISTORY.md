# Rerail rule pack: Rerail repository, history

These are the rules that were considered for the pack, and the study behind each. This file is not published; the
pack's [README](../README.md) covers the rules it ships, and [candidates/README.md](candidates/README.md) the two drafts
that were not approved.

The candidates were drafted, checked and shipped: 34 rule documents in [`../rules/`](../rules/), 14 in Steer and 20 in
Log, where a candidate whose parts needed different modes became two. Every document passes `rerail rules test`
(valid, every case right, none unchecked) and was run against the history below. Two drafts were not approved and are
listed under [Considered and not proposed](#considered-and-not-proposed). This pull request also fixed most of the
code the candidates found; each section's "Today" says what was fixed and what is left.

## How the candidates were found

A rule is worth steering on when this repository has broken it and had to correct it. Each candidate below was checked
against the whole history of `neralabs/rerail` as of 2026-10-30: every branch, the head of each of the 286 pull
requests, the unreachable pre-squash commits, and the 1,874 review and conversation comments on them. "Today" in the
history columns is `main` at `c39847fb1` (#353) for the first round and at `719a600d0` for the [second
round](#second-round-design-docs-and-the-six-shapes). "Now" is what the shipped rule matches in this repository after
this pull request's fixes.

The counts in the tables:

| Column | Counts |
|---|---|
| Main | Breaks that reached `main` after the convention existed |
| Today | Of those, and of older breaks never corrected, how many were still on `main` before this pull request |
| Now | Matches of the shipped rule's repository pass after this pull request |
| Branch | Breaks written on a pull request branch and corrected before it merged |
| Review | Review comments, by people or `neralabs-ci`, that flagged a break |

A break from before its convention existed is counted separately and is not held against the rule. The first round
proposed candidates from three sources: REVIEW.md's merge blockers, statements in CONTEXT.md, `src/README.md` and
`docs/` that one module owns a job, and the corrective commits and review comments in history. The second round
followed open PR #228's method for architecture rules: folder READMEs, module and file-head doc comments, and the six
shapes it names.

Apple Git 2.54 reads `\s`, `\b`, `\w` and `\d` literally in `git log -G` and `git grep -E`, which silently returned
no matches for some searches. Every search in both rounds was re-run with `git grep -P` or POSIX classes; the counts
below are the corrected ones.

The modes the rules shipped in:

- **Steer**: the code holds the rule today, so a new break is steered.
- **Log**: today's code still breaks the rule and the fix is a refactor nobody has scheduled, or the rule should run a
  while first. Switch it to Steer once the matches are gone.

## Summary

| Proposed id | Rule | Main | Today | Branch | Review | Shipped | Now | Kind |
|---|---|--:|--:|--:|--:|---|--:|---|
| `rerail-language-branching` | Only an Engine Adapter branches on a language or family | 3 | 3 | 4 | 3 | Log, until the 4 branches below move onto the adapter; grammar node kinds ship as `rerail-grammar-node-kinds`, Log | 4; 11 | code, Rust and TypeScript |
| `rerail-pack-rule-test-placement` | A test of one pack rule lives in `test/engine/code-rule/pack-rules/` | 8 | 8 | 4 | 0 | Steer | 0 | code, path |
| `rerail-pack-data-through-publish` | Pack data reaches S3 and the locks only through the publish workflow | 5 | 5 | 0 | 0 | Steer | — | conversation rule |
| `rerail-ui-color-tokens` | The dashboard defines no raw colours; every colour is a Nera token | 21 | 6 | 2 | 0 | Steer, with an allow-list | 0 | code, CSS and JS |
| `rerail-provenance-writers` | Licence and provenance records come only from their shared writers | 7 | 0 | 4 | 3 | Steer | 0 | code, scripts |
| `rerail-no-message-classification` | Failures are classified by code, never by their message text | 6 | 5 | 1 | 6 | Log | 14 | code, Rust and TypeScript |
| `rerail-workflow-script-facts` | Workflows don't restate what a script owns | 3 | 1 | 1 | 6 | Log, until no workflow tests a pack's name; the shared-condition part ships as `rerail-workflow-shared-conditions`, Log | 2; 6 | code, YAML |
| `rerail-pack-data-runtime-names` | Pack-data recipes take runtime names from `gates.yaml` | 8 | 5 | 5 | 5 | Steer; the helper-copy part ships as `rerail-pack-data-helper-copies`, Log | 0 (was 2); 8 | code, scripts |
| `rerail-cli-entry-no-decisions` | A script that runs on import holds no decision logic | 2 | 0 | 4 | 6 | not shipped | — | code, scripts |
| `rerail-rule-documents-in-packs` | A rule document is written only inside a pack | 0 | 0 | 2 | 1 | Steer | 0 | code |
| `rerail-language-facts-from-registry` | Language facts come only from the language registry | 2 | 1 | 3 | 3 | Log, as its own rule | 3 | code, Rust and TypeScript |
| `rerail-generator-placement` | A script that writes committed files lives under `scripts/generate/` | 0 | 4 | 1 | 2 | Steer | 0 (was 4) | code, path |

## Candidates

### `rerail-language-branching`: only an Engine Adapter branches on a language

- **Stated in**: CONTEXT.md, Language Registry and Engine Adapter ("apart from each family's Engine Adapter, nothing
  branches on a language"), since #2 (`fbb6104fc`) and #33 (`abf63d8a5`); a merge blocker in REVIEW.md since #76
  (`985818996`). Nothing checks it mechanically.
- **History**: #33 and #44 moved 113 branch sites from the original port into the adapter. After that, three new
  branches reached `main` and four were caught on a branch: three in #52 (fixed in `5f2880a54`) and one in #132 (fixed
  in `8dfb1e33d`), each within an hour of the review comment. Thirteen more sit on unmerged branches. The
  `/rerail-create` run in #119 already suggested this rule first.
- **Today**: not fixed by this pull request, so the rule ships in Log. Still on `main`:
  - `native/src/code_rule_dsl/compile/import_target.rs:89` `if Family::of(language) != Family::Ecma {`
  - `native/src/code_rule_dsl/tape.rs:1170` `if grammar.lang == GrammarDialect::Blade {` (from `aac1ff237`, #32)
  - `native/src/code_rule_dsl/tape.rs:1278` `if grammar.lang != GrammarDialect::Blade {` (from `1fbb06c7c`, #132)
  - `native/src/code_rule_dsl/workspace/evaluate.rs:856` `let image = if language == GrammarDialect::Blade {` (from
    `aac1ff237`, #32)

  The registry already declares Blade's parser (`"parser": "forte"`), and the engine does not read it.
- **Shape**: a code rule over `native/src/**/*.rs` and `src/**/*.ts`, forbidding comparisons, `match` arms and
  `matches!` on `Family::`, `GrammarDialect::` or a language name, outside the adapter: `language/adapter*` and the
  per-family modules the adapter's module documentation names (`scope/<family>.rs`, `value/<family>/`, `calls/…`).
- **Answered**: whether the node-kind checks in `native/src/code_rule_dsl/suppress/rerail.rs:116,196,230` count as
  branching on a language. #50's review asked and nobody answered; on
  [#373](https://github.com/neralabs/rerail/pull/373#issuecomment-6606280318) Timo answered that they do: node kinds
  are per-grammar names. PR #400 moved them onto each family's adapter, and the rest are flagged by
  [`rerail-grammar-node-kinds`](#rerail-grammar-node-kinds-grammar-node-kinds-only-on-the-familys-adapter), a separate
  rule because this one steers and that one has to start in Log.

### `rerail-grammar-node-kinds`: grammar node kinds only on the family's adapter

- **Rule**: shared engine code names no tree-sitter node kind; the kinds sit on the family's Engine Adapter
  (`Adapter::COMMENT_KINDS`, `PREAMBLE_KINDS`, `BODY_KINDS`, `ScopeRules::REFERENCE_KINDS`, `ValueRules::VALUE_KINDS`
  and the like) and are read through `with_adapter!`. Code that runs only after its family is chosen may name them.
- **Stated in**: CONTEXT.md, Engine Adapter, as answered on the #373 review: "it's branching: node kinds are
  per-grammar names". #50's review raised the same lines as a follow-up.
- **History**: #50 merged `suppress/rerail.rs` with comment kinds of five grammars, PHP's and ECMAScript's preamble
  tokens and Python's `block` in shared code; the review comment asking to move them went unanswered. PR #400 moved
  them onto the adapter.
- **Today**: the shapes matched 1,288 places before #400 (`871c0661c`) and 1,285 on #400's head (`a0e55ecff`): 917
  in family-named modules, left out by name, and the rest triaged below. The shipped rule matched 14 before #400; #400
  moved the 3 in `suppress/rerail.rs` and the other 11 stay, so it matches 11 now. Every match outside family-named
  modules, by verdict (line numbers are the same on both trees outside `suppress/rerail.rs`):

  | File | Matches (here / #400) | Verdict | How the rule treats it |
  |---|--:|---|---|
  | `suppress/rerail.rs` (`is_comment` 116, `first_code` 196, `covered` 230) | 3 / 0 | Shared; moved by #400 | Matched |
  | `compile/resolve.rs` (`compile_binding_in` 442; `compile_lexical_resolves` 612, 633, 635, 642, 644) | 6 | Shared: Python, Ecma, PHP, Rust and SQL reach them | Matched |
  | `matcher/arguments.rs` (`named_children` 64, `unparenthesized` 504) | 2 | Shared: Python, PHP, Ecma and Rust callers | Matched |
  | `matcher/compare.rs` (`element_index` 314, `unparenthesized` 336) | 2 | Shared: `index` and `commute` run for every family | Matched |
  | `matcher/util.rs` (`is_comment_kind` 38) | 1 | Shared: comment kinds of four grammars, for every family | Matched |
  | `compile/resolve.rs` (`compile_php_resolves`) | 2 | After dispatch: PHP (`S::FLOW`) | Left out: function named for PHP |
  | `compile/parse.rs` (`compile_parse`) | 5 | After dispatch: one `LiteralHost` arm per family | Left out: arm on a family-named variant |
  | `language.rs` (`KindCatalog::kind_ids` 158) | 1 | After dispatch: the `OwnKinds::Blade` arm | Left out: arm on a family-named variant |
  | `language.rs` (module-syntax and JSX-runtime helpers) | 25 | After dispatch: Ecma (`source_type` `None`, `JSX_RUNTIMES`) | Left out: listed by name |
  | `matcher/arguments.rs` (`python`, `syntactic`, `literal_keywords`, `php`, `php_syntactic`, `php_arguments`, `typescript`; `match_node` 81) | 22 | After dispatch: each family's `ArgumentRules`; line 81 behind the `estree` flag only Ecma accepts | Left out: named for the family, listed by name, or inside `if self.estree` |
  | `matcher/format_args.rs` (`prepare_python` and its helpers; `prepare_rust`) | 39 | After dispatch: the Brace and Percent styles (Python) and the Rust style | Left out: named for the family or listed by name |
  | `matcher/resolve.rs` (`this` helpers; `seenBefore` and import helpers; `php_holds`) | 35 | After dispatch: Ecma's `this_global`, Python's `imported`, PHP's `S::FLOW` | Left out: named for the family or listed by name |
  | `scope/usage.rs` (`dunder_all` and its helpers) | 22 | After dispatch: Python (`Usage::python`) | Left out: listed by name |
  | `calls/summary.rs` | 41 | After dispatch: only `impl CallRules for Python` | Left out: file |
  | `value/structure.rs` | 36 | After dispatch: only `impl ValueRules for Python` | Left out: file |
  | `value/key.rs` | 37 | After dispatch: only `impl ValueRules for Ecma` and `value/ecma/` | Left out: file |
  | `module_graph.rs` | 12 | After dispatch: `A::PACKAGE_MANIFESTS`, true only for Ecma | Left out: file |
  | `import_target/term.rs` | 7 | After dispatch: `importTarget` compiles for Ecma only | Left out: folder |
  | `embed/test_script.rs` | 7 | After dispatch: a test-only HTML embedder | Left out: file |
  | `literal/annotation.rs`, `offsets.rs`, `semantic.rs` | 26 | After dispatch: Python strings and annotations | Left out: files |
  | `literal/brace.rs`, `matcher/format_args/printf.rs`, `scanf.rs` | 11 | Rerail's literal grammars' kinds; PHP's and Python's styles | Left out: files |
  | `suppress/` linter readers (biome, clippy, eslint, noqa, nosec, oxlint, phpcs, phpstan, psalm, sqlfluff) | 29 | After dispatch: each is read only by its family's `suppresses` | Left out: files |

  Each verdict was traced from the function's callers to the family's adapter impl (`language/adapter*.rs`), a
  family-named module, or a compile-time gate only one family passes.
- **Shape**: a code rule over `native/src/**/*.rs` matching node kinds compared with, matched against or listed for a
  node's kind, or passed to `kind_ids`, outside the adapter, family-named modules and the modules and functions above.
  Punctuation and `ERROR` pass. It ships in Log; switch it to Steer once the 11 shared matches move.

### `rerail-pack-rule-test-placement`: a pack rule's test lives in `pack-rules/`

- **Stated in**: nowhere. The only guard is structure metric 12, the 25-file limit on `test/engine/code-rule/`, which
  says nothing about placement.
- **History**: seven pack-fix pull requests each passed CI alone. Merged together, they put 26 files in
  `test/engine/code-rule/` and turned `main` red (CI job 116088383298) for 30 minutes. The repair was made four times
  (`d8f3bd524`, `10f9386b3`, `8cb32875c`, `ae46726be`). #106 then moved the same files to `languages/`, 17 hours
  after `pack-rules/` had reached `main`. Four more flat tests were written on branches and moved before merge (#68,
  #71, #72, #74).
- **Today**: fixed in this pull request. The seven duplicates in `test/engine/code-rule/languages/` are deleted and
  `typescript-class-literal-property-style.test.ts` (#67) moved to `pack-rules/`. The rule excludes
  `test/engine/catalog/javascript-rule-options.test.ts`, one of the two borderline files.
- **Shape**: a code rule over `test/**/*.test.ts` outside `pack-rules/`, matching a test that loads one pack rule
  document. The steer moves the file to `pack-rules/`, or deletes it where a copy is already there.

### `rerail-pack-data-through-publish`: pack data goes through the publish workflow

- **Stated in**: CONTRIBUTING.md:547-551 (`fixtures:push` is for maintainers repairing S3 outside a pull request),
  REVIEW.md, "Approving a pack data upload". The convention exists since #177 (`c8352a77c`, 2026-10-27), which added
  `pack-data-publish.yml`.
- **History**: #208 put its locks back to `main`'s, then pushed directly again 80 minutes later, and merged with a note
  that the uploads "skip the workflow's code-owner approval" (`caf6da36f`, `686100276`, merged as `005ff0041`). #338
  pushed the lizard data the same way (`dd87a1c13`, merged as `69a9373dd`). No review flagged either. The publish bot
  has never set a lock on `main`.
- **Today**: the five locks still name archives uploaded from a workstation, each at line 3; the shipped conversation
  rule steers new pushes and does not read the locks.
  - `packs/react/upstream/eslint-plugin-react/test-data.lock`
  - `packs/react/upstream/eslint-plugin-react/samples.lock`
  - `packs/react/upstream/eslint-plugin-react-hooks/test-data.lock`
  - `packs/react/upstream/eslint-plugin-react-hooks/samples.lock`
  - `packs/complexity/upstream/lizard/test-data.lock`
- **Shape**: a command rule on `npm run fixtures:push`, `pack-fixtures.mjs samples push` and `upload`, steering to
  staging into `publish/` and dispatching the publish workflow. Every break used one of those commands.

### `rerail-ui-color-tokens`: the dashboard defines no raw colours

- **Rule**: in `ui/**`, every colour is a token, a Nera token where one fits. `ui/tokens.generated.css`, generated from
  the `web/src/index.css` of Nera's design-system source, holds the Nera tokens. A local token may alias a Nera token;
  a local token whose value is a raw colour, a raw colour in a declaration, and a redefinition of a Nera token's name
  are breaks.
- **Stated in**: nowhere. `scripts/check/check-ui-tokens.mjs` checks only that every `var(--…)` the page uses is
  declared.
- **History**: #196 (`6c84dada1`) brought 21 raw colours into `ui/css/base.css` as local tokens, two of them in
  branch commits `0a3ec53d5` and `30ded9c46` that replaced Nera tokens (`--steered`, `--logged`) with raw hex. #353
  replaced 19 with Nera tokens six days later. On #209 a raw `--header-primary` overriding the Nera token was removed
  within half an hour. No code review flagged any of them; the designer's token review on the dashboard tickets did.
- **Today**: unchanged. Six local tokens hold raw colours, all black alphas for shadows and the scrim, which the header
  comment in `base.css` and #353 call deliberate "named effects"; the rule allows them by name, so it matches none:
  - `ui/css/base.css:52-54` `--lift`, `--scrim`, `--overlay-shadow`
  - `ui/css/base.css:72-74`, `:86-88`, `:99-101` `--card-shadow`, `--panel-shadow`, `--bar-shadow` in each theme
- **Shape**: a regex code rule over `ui/**/*.{css,js,html}` except `tokens.generated.css`, forbidding `#hex`, `rgb(`,
  `rgba(`, `hsl(`, `hsla(`, `oklch(` and named colours, with the six effect tokens allowed by name; and a second clause
  forbidding a Nera token's name declared outside the generated file.
- **Open question**: whether the shadow and scrim colours should become Nera tokens upstream, so the allow-list goes.

### `rerail-provenance-writers`: licence and provenance records come only from their writers

- **Rule**: only `scripts/pack-data/provenance.mjs` writes `provenance.json` and a tool's `LICENSES.md`; nothing writes
  `licenses/<project>/UPSTREAM.md` except `scripts/upstream/bump.ts` moving its Source row; only
  `scripts/s3/pack-fixtures.mjs` parses "Not copied"; only `scripts/samples/licences.ts` decides a sample's licence and
  holds the copyleft lists.
- **Stated in**: docs/pack-format.md, docs/upstream.md and CONTRIBUTING.md describe the records; who owns them is
  unwritten.
- **History**: a hard-coded `RUFF_NOT_COPIED` list missed three copyleft directories (#85 stack). After #177 moved the
  convention, six generators and Ruff's own "Not copied" parser still wrote and read the old records; #234
  (`1e7042bf7`) fixed all seven a day later, and derive and upstream bumps stopped for that day. On #177 itself, four
  breaks were fixed before merge, among them samples trusting their own licence label, which would have let a GPL file
  reach S3 as MIT. Review comments 6274453353, 4389693506 and 4389693515.
- **Today**: none.
- **Shape**: a regex code rule over `scripts/**`, `conformance/**` and `packs/*/upstream/**`, forbidding writes to those
  file names, a "Not copied" pattern and a copyleft array outside the owners. Writes to a tool's own
  `upstream/<tool>/UPSTREAM.md` are allowed.

### `rerail-no-message-classification`: failures are classified by code, not message text

- **Rule**: code that decides what kind of failure it has reads a code or one shared constant, never the wording of
  another component's message. Hosts build the steer notice from `delivery.rules`, never by parsing the text sent to
  the agent.
- **Stated in**: docs/in-session-surfaces.md:61 and :86 for the steer notice ("nothing parses it"), since 2026-10-21.
  The engine-message half is unwritten.
- **History**: review first named the problem on 2026-10-14. Six more cases reached `main` after that, and only one was fixed
  (#162). Recorded harm: a spoofable `why?` link (#61), a transient storage error that dropped a gap record for good
  (#46), and a dashboard stuck on "Unknown repository" because it matched "answered 404" (#129). In #45 the failure
  codes were too coarse, so the fix matched wording and deleted a note that said "classified by failure code alone,
  never message text".
- **Today**, not fixed by this pull request; the shipped rule matches 14. After the principle was stated:
  - `src/engine/code-rule/rerail-steer.ts:101` (and `:40`), called from `src/harness/hook-adapter.ts:167`
  - `src/engine/audit/audit-control.ts:27` `/^Repository scan stopped after reaching \d+ failures$/u`
  - `src/engine/audit/audit.ts:156` `/^\d+ further failures? not recorded$/u`
  - `src/harness/native-session-client.ts:653` `/Unknown session|busy|locked/iu`
  - `native/src/code_rule_dsl/workspace/results.rs:347` `"file has syntax errors"` (review r630, open)

  and from before it: `src/engine/audit/audit.ts:141`, `src/harness/event-stream.ts:49`,
  `src/harness/native-session-client.ts:657,663-665` and `native/src/code_rule_dsl/workspace/persisted.rs:153`.
  `src/setup/updater.ts:305`, which reads an older release's error, is an accepted exception. Also from before it, and
  a judgement call: `src/engine/code-rule/code-rule-enforcer.ts:1337,1820` compare `error.message` with
  `"WORKSPACE_BUSY"`, a literal the Rust side spells separately. (Line numbers are from `719a600d0`.)
- **Shape**: a regex code rule over `src/**` and `native/src/**` matching `.message.startsWith(`, `.includes(` or a
  regex test on a message, and duplicated engine message literals. Log first, until the failures carry codes.

### `rerail-workflow-script-facts`: workflows don't restate what a script owns

- **Rule**: a workflow reads what core ships from `scripts/release/core-paths.mjs`, built entry files from
  `ENTRY_POINTS`, tool-pack legs from the plan scripts, pins from `sync-pins.mjs` and Windows host steps from
  `scripts/host/windows-host.ps1`; it does not copy them. A condition several jobs share is decided once.
- **Stated in**: docs/environment-matrix.md, docs/release.md and `src/README.md:29-31` name the owners. The
  shared-condition part is unwritten.
- **History**:
  - #51 (`6eae62d80`): release run 38440166792 failed with `missing dist/cursor-hook.js`, because `release.yml` still
    named the flat paths #26 had moved. Pull request CI never runs that step.
  - `env-matrix.yml` `paths-ignore` (#204), fixed in #211 a day later.
  - #85 hand-listed a 20-leg matrix, replaced by `plan.mjs` before merge.

  Six review comments flagged it: 4274743590, 4296436491, 4296436480, 4379796855, 4379796895 and 4381032053.
- **Today**: unchanged, so the rule ships in Log. `.github/workflows/upstream-bump.yml:161` still tests
  `matrix.pack == 'php' || matrix.pack == 'blade'` to set up PHP, and no planning step emits the runtime a leg needs.
  The release pull request skip `github.head_ref != 'changeset-release/main'` is still written once per job, seven
  times in `.github/workflows/ci.yml` and once in `.github/workflows/release-packs.yml:65`.
- **Shape**: a regex code rule over `.github/workflows/**` forbidding `dist/…-hook.js` literals, `paths:` lists that
  copy what core ships, and hand-listed `pack:` entries or `matrix.pack == '<id>'`. The shared-condition clause (at most
  one release pull request skip per file) shipped as `rerail-workflow-shared-conditions` in Log, until `ci.yml` decides
  it once.

### `rerail-pack-data-runtime-names`: recipes take runtime names from `gates.yaml`

- **Rule**: a pack-data recipe or recorder reads its runtime from `gates.yaml` `upstream.runtime` and uses the shared
  helpers in `scripts/pack-data/`; it never builds a versioned runtime name or keeps its own copy of a helper.
- **Stated in**: docs/upstream.md:125 and :351 for the runtime; the helper ownership is unwritten.
- **History**: a literal versioned runtime name was fixed on `main` five days after it landed, and `complexity.ts`
  built a new one the day after. Five more breaks were fixed on branches (#40, #155 twice, #85, PR #22's branch).
  Review comments 6150967373, 4231223646, 6274453353, 4339536970 and review 5588921453.
- **Today**: this pull request fixed the two recipes: `python.ts` (`` uvRuntime(`${tool}-${pins.version}`, … ) ``) and
  `complexity.ts` (`` uvRuntime(`lizard-${version}`, … ) ``) now read `upstream.runtime` from the tool's `gates.yaml`.
  Still on `main`:
  - `packs/javascript/upstream/generate.mjs:111,120` and `packs/typescript/upstream/generate.mjs:128,137` keep their
    own `codePointColumn` and `lint`.
  - `ECMA_EXTENSIONS` is kept four times, with no shared owner since 2026-10-27.
- **Shape**: a code rule over `scripts/pack-data/recipes/**` whose runtime argument may be no literal or template other
  than the unversioned `python`, `php`, `blade` or `parsers`. The helper-copy part shipped as
  `rerail-pack-data-helper-copies` in Log.

### `rerail-cli-entry-no-decisions`: a script that runs on import holds no decision logic (not approved)

- **Rule**: a script whose top level does work when imported only parses arguments and does I/O; what it decides lives
  in a module beside it that can be imported and tested (`scripts/release/packs.mjs`, `core-paths.mjs`,
  `src/runtime/*`, `eslint-record.mjs`).
- **Stated in**: nowhere but commit messages (#83) and review (r855, r1001).
- **History**: two cases reached `main` and were fixed (#57's `packs-promote.mjs`, fixed in #147 after 6 days; the
  vitest recorder's check from #41, fixed in #83); four were moved before merge (#147, #211, #133, #241). Twice the
  review bot asked for the logic inline in the script, and it was moved out later. Seven bugs were fixed in decision
  logic that stayed in such scripts.
- **Today**: no break by the rule's narrow form. Untested decisions still sit in `scripts/host/azure/matrix.mjs:205`
  `judge()`, `src/cli/cli.ts:573` `isVersionIndependentCommand`, `scripts/release/release-version.mjs` and
  `scripts/release/packs-promote-plan.mjs`.
- **Shape**: a code rule over `scripts/release/**`, `scripts/host/**`, `src/cli/cli.ts` and the recorders, firing when
  a script that runs on import gains a branching function it does not export. Generators are Log.
- **Not approved**: the drafted rule catches none of the six recorded breaks, which were all written inline rather than
  as a separate function, and about half its matches are code it should leave alone. See [Considered and not
  proposed](#considered-and-not-proposed).

### `rerail-rule-documents-in-packs`: a rule document is written only inside a pack

- **Rule**: a rule document (`rerail.policy/v1`, a conversation rule, or a `{ "kind", "rule" }` draft) is added only
  under `packs/<id>/rules/`, or as a test input under `fixtures/`.
- **Stated in**: docs/repository-structure.md metrics 02 and 23. Metric 02 recognises only a top-level
  `"format": "rerail.policy/v1"`, not a conversation rule or a draft.
- **History**: both breaks were agents writing Rerail's own rules, which is the work this pack is for. #176 put
  `packs/*.draft.json` at the root of `packs/` (`a9fef8062`) until review 4341135728 asked for `packs/rerail`;
  #183 put a conversation-rule draft under `fixtures/host/`. Each was fixed within about 90 minutes.
- **Today**: none.
- **Shape**: a code rule over `**/*.json` outside those folders matching a rule's shape. The structure check should
  also learn the two shapes metric 02 misses.

### `rerail-language-facts-from-registry`: language facts come only from the registry

- **Rule**: file suffixes, families, dialects and grammar names come from `languages/<id>.json` and the tables
  generated from it; no hand-kept list.
- **Stated in**: CONTEXT.md, Language Registry ("The one list of the languages Rerail knows").
- **History**: on the #33 branch the Rust `SOURCE_SUFFIXES` missed `.vue`, so the Git index skipped files the host
  reads; the fix patched the list instead of removing it. ECMA extension tables were written outside the adapter twice
  in #52 and again on open PR #207. A merge on #44 dropped `javascript` from a hand-kept test list.
- **Today**: unchanged; the shipped rule matches 4. `native/src/git_workspace.rs:27` `SOURCE_SUFFIXES`; and, as a
  judgement call, `native/src/code_rule_dsl/module_graph.rs:75` `EXTENSIONS` and `:446-449`, which reached `main`
  through #52 though review finding F1 asked for the registry.
- **Shape**: a regex code rule flagging a list or match of two or more registry suffixes outside the adapters,
  generated files and tests. It shipped as its own rule in Log rather than as a clause of `rerail-language-branching`;
  generate `SOURCE_SUFFIXES` from the registry in `build.rs` before switching it to Steer.

### `rerail-generator-placement`: generators live under `scripts/generate/`

- **Rule**: a script that writes committed files lives under `scripts/generate/<area>/` and has a row in
  `scripts/generate/README.md`, never beside its output.
- **Stated in**: `scripts/generate/README.md:3-4`; structure metric 06, which matches only file names starting with
  `generate`.
- **History**: #26 moved 13 generators. A vitest policy builder in #41 was moved before merge (`a9d0adc3d`). In #50,
  21 React rule documents gained `suppress.oxlint` without a change to the builder, which was fixed 12 days later
  (`500790c67`): the outputs are not listed as generated, so the review rule against hand edits never applied.
- **Today**: fixed in this pull request. The four builders from before the convention moved to
  `scripts/generate/packs/`: `next-policies.mjs` and `next-probes.mjs` (from `packs/next/candidates/build-policies.mjs`
  and `build-probes.mjs`), `react-policies.mjs` and `react-specs.mjs` (from `packs/react/candidates/build.mjs`), and
  `react-probes.mjs` (from `packs/react/candidates/probes/build.mjs`).
- **Shape**: a code rule over `packs/**` and `native/src/**` script files that call a file-write function, steering to
  `scripts/generate/<area>/` and a README row.

## Second round: design docs and the six shapes

Eight discovery passes read the 17 folder READMEs and the design notes in `docs/`, the `//!` module docs of 250 Rust
files, the file-head comments of `src/`, `scripts/`, `conformance/` and `ui/`, and searched the code through the six
shapes in PR #228's `references/architecture-rules.md`: dependency direction, layer purity, a construct in the wrong
place, a cross-cutting change in a shared seam, a policy decided in the wrong layer, and one home per thing. Their
candidates, merged where they overlapped, were checked against history like the first round's. "Today" here is
`719a600d0`.

Most of them hold today and have never been broken; those are in [Considered and not
proposed](#considered-and-not-proposed). The ones below have history behind them.

| Proposed id | Rule | Main | Today | Branch | Review | Shipped | Now | Kind |
|---|---|--:|--:|--:|--:|---|--:|---|
| `rerail-home-file-names` | A file in the Rerail home is named only by the module that writes it | 7 | 6 | 1 | 2 | Log on hand-kept name lists (`src/engine/storage/private-files.ts` can't import the writers' constants); the re-spelled paths ship as `rerail-home-file-paths`, Log | 1; 11 | code, TypeScript |
| `rerail-walk-limits` | Every repository walker takes its ignored folders and limits from the enforcer | 6 | 5 | 3 | 2 | Steer in `src/`; `rerail-walk-limits-native`, Log, in `native/` | 0; 3 | code, TypeScript and Rust |
| `rerail-conformance-boundary` | `conformance/` runs programs only through `process.ts`, imports nothing from the authoring path, and reads only pinned data | 3 | 0 | 13 | 0 | Steer on the first two; pinned data ships as `rerail-conformance-pinned-data`, Log | 0; 0 | code, TypeScript |
| `rerail-fixtures-root` | Rust tests reach `fixtures/` only through `fixtures_dir!()` and `oracle!()` | 1 | 18 lines | 0 | 0 | Steer | 0 (was 18 lines) | code, Rust |
| `rerail-command-sigil` | The command sigil (`$` in Codex, `/` elsewhere) is decided in one place | 9 | 6 | 0 | 0 | Steer | 0 (was 6) | code, TypeScript |
| `rerail-git-no-lazy-fetch` | Every Git process Rerail starts has lazy fetch off | 0 | 1 | 1 | 2 | Steer | 0 (was 1) | code, TypeScript |
| `rerail-no-code-execution` | Reading a repository's configuration or a pack's match definition runs no code, and npm runs with `--ignore-scripts` | 0 | 0 | 3 | 5 | Log | 0 | code, TypeScript |
| `rerail-origin-ranges` | A byte range that crosses documents is mapped only through `origin.rs` | 0 | 0 | 1 | 1 | Log | 0 | code, Rust |
| `rerail-pack-package-names` | A pack's npm name is built only by `rulePackPackageName` | 7 | 7 | 1 | 0 | Steer, since #340 landed | 0 (was 1) | code, TypeScript |
| `rerail-fixed-value-lists` | A fixed value list (hosts, audit scopes and modes) is declared once and used through its constant | 12 | 12 | 0 | 0 | Log | 15 | code, TypeScript |

### `rerail-home-file-names`: the Rerail home's files are named by their writers

- **Stated in**: nowhere. Review 4383409850 on #222 asked to "build the list from the constants the writers already
  use, so it cannot drift".
- **History**: uninstall's hand-kept list of home files came in with #222; its first draft lacked four names and had
  two stale ones, fixed 23 minutes later (`89719cc85`), with the hand list kept. #241 (`12d769ab9`) then added the
  daemon's `access-token` (`src/runtime/daemon-control.ts:47`) without adding it to the list.
- **Today**: the live bug is fixed in this pull request. Uninstall's home-file list is now built from the writers'
  constants and includes the daemon's `access-token` (`DAEMON_ACCESS_TOKEN_FILE`), so the hand-kept
  `RERAIL_HOME_ENTRY` pattern is gone, and `src/setup/harness-registration.ts` builds its list from the same constants.
  `src/engine/storage/private-files.ts` keeps a hand list, because the engine cannot import the `runtime/` and `setup/`
  constants, so the rule ships in Log. Still open, and matched by `rerail-home-file-paths` in Log (11 today): paths
  re-spelled with `join` instead of the owner's constant, at `719a600d0` `src/setup/harness-registration.ts:390`,
  `src/setup/coding-agents.ts:39`, `src/setup/rule-pack-store.ts:174`, `src/engine/catalog/rule-catalog.ts:58`,
  `src/cli/cli.ts:238`, and from before: `harness-registration.ts:200`, `src/engine/native/rerail-build.ts:24,48`.
- **Shape**: a regex code rule over `src/**` flagging a literal or pattern that names two or more home files outside
  their owners (log), and `join(…, "current" | "versions" | "update-state.json" | "bin")` outside the updater
  (`rerail-home-file-paths`, log: those names exist only as `Updater` getters, which `engine/` cannot import, so they
  need exported constants first).

### `rerail-walk-limits`: repository walkers share one set of folders and limits

- **Stated in**: nowhere. `src/cli/rule-draft.ts` says "The same bounds a repository scan walks with".
- **History**: in #52, review 4275893725 found the Rust module-graph walk ignoring the host's folders and limits, and
  review 4297815767 its missing entry cap. Each fix (`0a4090950`, `84a75b580`) copied the host's values instead of
  passing them. No copy has drifted in value yet.
- **Today**: fixed in `src/` in this pull request. The enforcer exports its walk limits (`AUDIT_MAX_FILES`,
  `MAX_SOURCE_BYTES`, `AUDIT_MAX_SOURCE_BYTES`) and `src/cli/rule-draft.ts` imports them; the TypeScript copies are
  gone. Still open, and matched by `rerail-walk-limits-native` in Log (3 today):
  `native/src/code_rule_dsl/module_graph.rs`'s own limits and folders. `src/engine/code-rule/code-rule-bridge.ts:24`
  keeps a separate 1 MB per-file limit, which is not the enforcer's (see [Still open](#still-open)).
- **Shape**: a code rule over `src/**` flagging a set literal holding both `"node_modules"` and `"__pycache__"`, or a
  `MAX_FILES`/`MAX_FILE_BYTES`/`MAX_SOURCE_BYTES` constant, outside the enforcer. In `native/` log only, pointing at
  `RepositoryIndexOptions`.

### `rerail-conformance-boundary`: conformance stays a judge

- **Stated in**: `conformance/README.md:17-18` and `:28-29` since #177; `conformance/process.ts`'s doc comment since
  `fce0f8827` (2026-10-14).
- **History**: #85 put direct `child_process` calls in `conformance/gates/cache.ts:2,14`, `conformance/gates/engine.ts:7,122`
  (`fce0f8827`) and `conformance/port-readiness/plan.mjs:12,124` (`30b01086c`); #177 (`c8352a77c`) removed them eight
  days later. #177's branch also imported from `scripts/pack-data` in recorders under `conformance/` (nine imports in
  four commits, fixed by `cbf682862`) and ran `aws s3 cp` directly. Three more direct calls sit on unmerged #160 and
  #175. The harm the reviews recorded: a corpus fetch that silently dropped tasks (4283765996), recorder code that let a
  fork choose its recorded answers (4330551098), and an `unzip` that reused partial trees (4377264352).
- **Today**: nothing in `conformance/`. As a judgement call, `conformance/ungated.ts:67` fetches the base commit with
  Git. Outside it, `scripts/pack-data/context.ts`, `derive.ts` and `eslint-expected.mjs` call Git directly although
  `process.ts` says `scripts/pack-data` runs programs through the runner. Not changed by this pull request.
- **Shape**: one code rule over `conformance/**` with two clauses: no `child_process` import outside `process.ts`; no
  import from `scripts/pack-data`, `scripts/upstream` or a pack generator, and nothing under `conformance/recorders/`.
  The third clause, no network fetch other than the pinned, checked one, shipped as `rerail-conformance-pinned-data`
  in Log.

### `rerail-fixtures-root`: Rust tests reach fixtures through the macros

- **Stated in**: structure metric 14 means to enforce one root resolver per language, but misses these: it looks for
  a relative `fixtures` literal only on lines with `join(`, `resolve(`, `URL(`, `Path::new(` or `PathBuf::from(`, not
  in `include_str!` or `format!`, and cannot see a direct `env!("RERAIL_ORACLES_DIR")`.
- **History**: a web application's import on #1's branch read fixtures through direct `env!` paths, fixed in `38efea8c4`
  and the merge `7e385006e`. #56 added relative `include_str!` paths that are still on `main`; four unmerged branches
  add 31 more lines, and one had to re-count `../` after a move (`3d260c7a8`).
- **Today**: fixed in this pull request. The 18 lines in `native/src/code_rule_dsl/scope/rust/tests.rs` and
  `scope/usage/rust/tests.rs` now read their fixtures through `oracle!`, the same root as their expected JSON.
- **Shape**: a regex code rule over `native/**/*.rs` (not `build.rs` or `code_rule_dsl/mod.rs`) flagging `../fixtures/`
  inside `include_str!`, `include_bytes!`, `format!`, `concat!` or `join`, and any direct `env!("RERAIL_ORACLES_DIR")`.

### `rerail-command-sigil`: the command sigil is decided once

- **Stated in**: nowhere.
- **History**: hard-coded `/` command names were fixed twice for Codex users: `042d82ebb` (#1) fixed in `457d97de8`
  (#61), and #25's `renderStoppedRepo` fixed in #222 (`e3f1c52cf`). Both fixes added another local
  `host === "codex"` check, so a rule written today would flag the fixes too.
- **Today**: fixed in this pull request. `ADAPTER_CAPABILITIES` (`src/engine/adapter-contract.ts`) holds a
  `commandSigil`, and the five sites that decided it (`rerail-steer.ts`, `repository-start.ts`,
  `rerail-command-files.ts`, `setup-screens.ts`, `cli.ts`) read it. The live bug is fixed too: `setup-screens.ts`'s
  `renderNotNow` (#182, which #222 missed) now prints `$rerail-start` to Codex users who answer "Not now".
- **Shape**: a regex code rule over `src/**` outside `src/hosts/**` flagging `=== "codex"` beside a `$` or `/` and
  `/rerail-` literals in agent-facing text.

### The other second-round candidates

- **`rerail-git-no-lazy-fetch`** (Steer). `docs/privacy.md:19` says blob reads run with `GIT_NO_LAZY_FETCH=1`. On
  #152, `f9722982f` added a `cat-file --batch` without it; `neralabs-ci` flagged it (F4) and `fce7c88cd` fixed it.
  Open before this pull request: `src/engine/audit/audit-control.ts:61-62` ran `git diff --cached --name-only` through
  `runGit`, which did not set the variable, and rename detection could then fetch a missing blob from the remote (seen
  with `GIT_TRACE` on Git 2.54). Fixed in this pull request: `runGit` sets `GIT_NO_LAZY_FETCH`, the audit's `git diff`
  passes `--no-renames`, and `src/setup/harness-registration.ts` runs Git through `runGit` instead of by bare name.
  Git honours the variable from 2.45.0 and in the 2024-04-19 security releases 2.39.4, 2.40.2, 2.41.1, 2.42.2, 2.43.4
  and 2.44.1; older Git ignores it (review F7). Rule: every Git spawn in `src/**` sets it, or goes
  through `src/engine/git.ts`, which sets it. Gabe's review of this pull request found the same rename failure in
  `src/cli/headless-scan.ts`'s `git diff`, reproduced in a sparse `--filter=blob:none` clone with an edited rename; it
  now passes `--no-renames` too, and the rule gained a clause for a `diff` or `status` argument list without it.
- **`rerail-no-code-execution`** (Log). On #140, `571fb8c21` ran each pack's match checks through `spawnSync` and
  `import()`, and uninstalled packs with npm scripts on; blocker 4315170665 and four more comments flagged it and
  `b2dcd0bb7` ("Packs run no code") removed it two hours later. Rule: no `node:child_process`, `node:vm`,
  `node:worker_threads`, computed `import(`, `eval` or `new Function` in `src/engine/catalog/**`,
  `src/setup/rule-pack-store.ts` and `src/cli/packs-cli.ts`; every npm install, uninstall, ci or update argument list
  includes `--ignore-scripts` and a `--` before specs. `src/setup/updater.ts:169-177` installs Rerail itself without
  the flag and would need it or an exception.
- **`rerail-origin-ranges`** (Log). `origin.rs:13-15`: "No caller maps a byte range itself." In the pull request that created
  it (#132), three callers still mapped ranges themselves through `segments.host_span`, which widened spans in Vue
  templates; blocker 4298111569 flagged it and `4e57c59d7` fixed it. Nothing on `main` breaks it; the unmerged
  `jeremybarnes/final-rerail-shape` adds `host_range` back (`d9ab616c5`). Rule: `\.host_(range|span)\(` and
  `fn host_(range|span)` in `native/src/code_rule_dsl/**` outside `origin.rs`, `literal/offsets.rs` and
  `embed/segments.rs`.
- **`rerail-pack-package-names`** (Steer, 0 today; was 1). #140 restated `@neralabs/rerail-pack-<id>` in six places
  instead of calling `rulePackPackageName`; #340 rewrote all but the folder join in `src/setup/rule-pack-store.ts` and
  added `packIdOfPackage` and `RULE_PACK_PACKAGE_NAME`; this pull request builds that folder from
  `rulePackPackageName` too.
- **`rerail-fixed-value-lists`** (Log, 15 today). #45 (`00983ec7f`) replaced three copies of the audit failure codes
  with one constant. Twelve lists were restated at `719a600d0`, two after their constant existed, among them the host
  list in `src/runtime/event-protocol.ts:9`, `src/setup/harness-registration.ts:39,41` and `src/cli/cli.ts:136`, and
  the audit scopes and modes in `src/engine/audit/audit-control-schema.ts:5,6,35` and
  `src/api/operations.ts:110-112,127-128`. None has drifted; this pull request moved none. Rule: a `z.enum([...])` or
  union restating a known list outside its owner.

The rest of the second round, each shipped as its own rule:

| Candidate | Shipped | Now | Today, and why it is not Steer |
|---|---|--:|---|
| Only `src/runtime/repository-start.ts` turns a checkout's status into a start state | `rerail-start-state`, Log | 7 | 7 copies at `719a600d0`, e.g. `src/api/repository-operations.ts:207-210`, `src/harness/hook-host.ts:56`, `src/hosts/omp/extension.ts:80,86`, `src/engine/catalog/rule-configuration.ts:406`; one more from #413. None was ever moved into the owner; one bug (review 4233535404) was fixed in place. The engine copies can't import the runtime owner |
| The dashboard shows the API's `*Source` fields instead of computing precedence | `rerail-ui-api-precedence`, Log | 10 | `ui/derive.js:66-76`, `ui/settings-writes.js:160-162,169`. #50's author fixed six precedence bugs in the dashboard's copy in one evening; the API can't yet say what a pack setting would decide |
| One helper resolves a session's storage root and model path | `rerail-session-paths`, Steer | 0 (was 13) | Fixed in this pull request: `src/runtime/session-paths.ts` resolves both, every copy routes through it, the daemon now reads both variables, and #221's folder-name guess (`src/runtime/runtime.ts:489`) is gone |
| Only `src/hosts/<host>/` imports a coding agent's SDK | `rerail-host-sdk-imports`, Log | 3 | `src/harness/native-semantic-tools.ts:1-3`. One break, unchanged since #26; move its two register functions into the OMP and Pi hosts first, or add package imports to `check-boundaries.mjs` (it skips them at `:150`) |
| A file is parsed into a tape only by `TapeImage::parse` | not shipped | — | See [Considered and not proposed](#considered-and-not-proposed) |
| Stored source and tape are read only in `workspace/documents.rs` | `rerail-store-reads-in-documents`, Log | 1 | `native/src/code_rule_dsl/workspace/evaluate.rs:164`. The only fix deleted the code; the facts redesign on unmerged branches stores tapes on purpose |
| `sqlite_master` table probes stay in storage | `rerail-table-probes-in-storage`, Log | 6 | 3 at `719a600d0`: `src/engine/catalog/setup-review.ts:30`, `src/engine/audit/audit-store.ts:42`, `src/runtime/repository-start.ts:40`; 6 on `main` now, one more from #413. Reviews asked for local probes as often as central ones |
| The dashboard's pure modules import no I/O module | `rerail-ui-pure-modules`, Log | 2 | `ui/repo-rules.js:7-8`. The README that lists it as pure came in the same pull request as the import |

## Considered and not proposed

| Rule | Main | Today | Branch | Review | Why not |
|---|--:|--:|--:|--:|---|
| `rerail-cli-entry-no-decisions`: a script that runs on import holds no decision logic (drafted, not approved) | 2 | 0 | 4 | 6 | The draft catches none of its six recorded breaks, which were all written inline, and about half its matches are code it should leave alone |
| `rerail-tape-parse-once`: a file is parsed into a tape only by `TapeImage::parse` (drafted, not approved) | — | 1 | — | — | The compiler enforces it better: route `encode_tape` (`native/src/code_rule_dsl/workspace/evaluate.rs:822`) through `TapeImage::parse` and make the encoders private. #48 and #132 each edited both copies |
| Only `src/engine/storage/` opens SQLite | 1 pull request (3 opens) | 3 | 0 | 0 | Never written down or flagged; open #351 deletes the three opens; `src/cli/project-cli.ts`'s pre-update copy looks deliberate |
| The package root comes from `src/engine/package-root.ts` | 0 | 0 | 0 | 0 | Never broken since #26; the one breakage was a workflow path (`rerail-workflow-script-facts`) |
| Shell text is quoted by one owner | 3 | 3 | 4 | 5 | Recurs, all on Windows, but the owners disagree with each other; consolidate the quoting first |
| Each hook entry is served by `serveHook` | 1 | 0 | 2 | 1 | Fixed in #61, unchanged since |
| A candidate's example pair is data in `conformance.json` | 4 | 0 | 0 | 2 | The schema now enforces it (#326) |
| A stored code rule is valid only once compiled | 0 | 0 | 2 | 1 | Both breaks on the day the convention began |
| Pack installs go through `installChecked` | 0 | 0 | 3 | 4 | All three inside the pull request that created it |
| Shared Rust helpers are reused | 0 | 0 | 2 | 1 | Two duplicates, both caught; a name-based check misses one |
| A migration step freezes its SQL | 0 | 0 | 1 | 1 | Once, caught; its own comments state the rule |
| Git is found without reading the environment | 0 | 0 | 1 | 2 | A test guards it; see the bare `git` call below |
| Dashboard links are built only in `daemon-control.ts` | 0 | 0 | 1 | 2 | Once, fixed before merge |
| History reads select by `historyRepositoryIds` | 0 | 0 | 1 | 1 | Once, fixed before merge |
| Attachment writes go through `queueInScope` | 0 | 0 | 1 | 1 | Once; two call sites in all |
| Set-up guards live in `setUpOneRepository` | 0 | 0 | 1 | 1 | Once; not mechanically checkable |
| Engine timing lines go through `timingLog` | 0 | 0 | 1 | 1 | Once; a test guards it |
| A host-only sandbox allowance stays out of the shared profile | 0 | 0 | 1 | 1 | Once, in test tooling |
| Core lists packs only through the generated list | 1 | 1 | 0 | 0 | One cosmetic badge map in `ui/render/pack-panel.js:45` |
| `e2e/` follows the test-root layout | 1 | 1 | 0 | 0 | A gap in the checks, not a rule: `e2e/` is missing from `check-boundaries` and metric 17 |
| Engine-layer purity: no `process.exit`, stdout or presentation writes; native matching reads no clock (second round) | 0 | 0 | 0 | 0 | Never broken; `src/harness/hook-launcher.ts:36` exits on purpose |
| API routes come only from the operation list; only `src/api` branches on the caller kind; structure metrics read only through `repo.mjs`; `schemas.ts` stays browser-safe; `hook-launcher.ts` imports only built-ins; `rule-documents.ts` stays a leaf; the internal-testing label has one spelling (second round) | 0 | 0 | 0 | 0 | Each owner has been the only home since it existed |
| Only `src/runtime/npm.ts` runs npm; only the release workflow and `packs-publish.mjs` publish; `rusqlite` only in `content_store`; only the daemon opens the content store; native code starts no process (second round) | 0 | 0 | 0 | 0 | Never bypassed in production; `rusqlite` outside the store is a deliberate direction on unmerged branches |
| The OMP bundle and the presentation reader never reach `better-sqlite3` (second round) | 0 | 0 | 0 | 0 | Never broken on 507 trees; it is cross-file, so a build check over the esbuild metafile fits better |
| A vendored grammar change updates its `PATCHES.md` (second round) | 0 | 0 | 0 | 1 | All 59 changes did; a CI job that regenerates `grammar.json` from the record fits better |
| `benchmark/` holds no committed results (second round) | 0 | 0 | 0 | 0 | Never on `main`; structure metric 3 already blocks most result files |
| Only `defaultRerailHome()` resolves `RERAIL_HOME` (second round) | 5 | 5 | 2 | 2 | No copy was ever replaced with the helper; a regex can't tell when a `home` is already in scope |
| Shared layers never compare a host id to a literal (second round) | 5 | 5 | 0 | 0 | All merged as written; the sigil sites are covered by `rerail-command-sigil` |
| UI: only `api.js` calls the network; views don't import `api.js`; only `views/index.js` imports views (second round) | 1 | 1 | 0 | 0 | `ui/views/settings.js:9` is the only break, never flagged |
| Rust module direction: support modules don't use `code_rule_dsl`; nothing outside `workspace/` imports it; nothing below `compile` imports it; Forte reaches out only through `tree.rs` (second round) | 0 | 0 | 0 | 0 | Breaks only on unmerged redesign branches with no pull request |
| One "rules on" count; rule modes compared only through `steers()` (second round) | 9 | 7 | 3 | 2 | Five definitions of the count disagree, so it needs a design decision first; the mode bugs were in the UI, which can't import `steers()` |
| Loopback hostnames in one place; one list of each agent's hook events (second round) | 5 | 5 | 1 | 0 | Never drifted into a bug; one-off refactors, not rules |

## Found on `main` while checking

These came up in the history checks. They are defects or check gaps, not rules. Fixed in this pull request:

- `src/setup/harness-registration.ts:721-722` ran `git` by bare name, through `PATH`, without `gitExecutable()` or
  its safe configuration; it now runs Git through `runGit`.
- `src/cli/setup-cli.ts:224-225`: uninstall's list of home files lacked `access-token`; the list is now built from the
  writers' constants (see `rerail-home-file-names`).
- `src/engine/audit/audit-control.ts:61-62` and `src/cli/headless-scan.ts:138` could make Git fetch from the remote in
  a partial clone; `runGit` sets `GIT_NO_LAZY_FETCH` and each `git diff` passes `--no-renames` (see
  `rerail-git-no-lazy-fetch`).
- `src/harness/setup-screens.ts:621` printed `/rerail-start` to Codex users (see `rerail-command-sigil`).
- #228's skill had two broken anchors.

Still open:

- `cleanupCommand` (`src/cli/setup-cli.ts:283`) quotes for PowerShell without handling the curly quotes the hook
  quoter handles, so its hint is wrong for a profile path holding ’.
- `scripts/check/check-boundaries.mjs:37` (`OUTSIDE_SRC`) and structure metric 17 leave out `e2e/`;
  `e2e/dashboard-test.ts` sits outside `e2e/support/`.
- `src/cli/rule-evidence.ts:302` builds a repository's history ids itself instead of calling `repositoryHistoryIds`.
- `native/src/git_workspace.rs:27` `SOURCE_SUFFIXES` is hand-kept (see `rerail-language-facts-from-registry`).
- `src/api/operation.ts:3-4` describes a generated web client that never existed; `docs/blade-parser.md:336` says
  `TapeImage::parse` is used for every caller, which `evaluate.rs` contradicts.
- The Python, TypeScript and JavaScript `PATCHES.md:19` name Blade (deleted in #32) and tree-sitter-generate 0.27.0
  (0.27.1 since #116); Python Patch 2's diff (`native/grammars/tree-sitter-python/PATCHES.md:87-141`) no longer applies
  with `git apply`.
- The browser-safety test of `src/engine/schemas.ts` covers 2 of its 8 files.

## Still open

- **`rerail-grammar-node-kinds` to Steer.** #400 moved the 3 in `suppress/rerail.rs`; it switches once the 11 shared
  matches in `compile/resolve.rs`, `matcher/arguments.rs`, `matcher/compare.rs` and `matcher/util.rs` move onto the
  adapter.
- **#176.** The open #176 also adds `packs/rerail`, with two rules on pack message wording; it has to be rebased onto
  this pack.
- **Per-file limits.** The bridge reads at most 1 MB per file (`src/engine/code-rule/code-rule-bridge.ts:24`), the
  enforcer 32 MB. `rerail-walk-limits` does not cover it.
- **Session paths.** The helper in `src/runtime/session-paths.ts` changed behaviour to confirm: an empty variable now
  counts as unset, which roots get a `.gitignore` changed, and the daemon now reads both variables.
- **Old Git.** Git before 2.45.0, apart from the 2024-04-19 security releases (2.39.4, 2.40.2, 2.41.1, 2.42.2, 2.43.4,
  2.44.1), ignores `GIT_NO_LAZY_FETCH` (review F7); `--no-renames` covers the audit's diff only. The partial-clone tests
  skip on such Git.
