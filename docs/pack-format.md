# Rule pack structure and validation contract

Status: the layout and the validation contract (checks 1 to 9) are implemented. `test/packs.test.ts` validates every
`packs/*` directory: `python`, `php`, `blade`, `typescript`, `react`, `node`, `next`, `tslint` and `complexity`. Core
does not load installed packs yet: it ships the TSLint and complexity packs inside its own package
(`packs/<id>/pack.json` and `packs/<id>/rules/` in its `files`, read from `CORE_RULE_PACKS` in
`src/rule-bootstrap.ts`; see [rule packs](rule-packs.md)), and no pack is published. A pack introduces no new file
format: it combines the two schemas the repository already has. To start one, see
[language packs](language-packs.md#start-a-pack).

Core's own built-in rules are not a pack: they would live in `code-rule-dsl/rules/` beside the engine's
`capabilities.json` and be seeded enabled. None ship today. Packs hold every other rule document; their rules are
seeded disabled.

## Principles

1. **A pack is data.** Rule documents plus the evidence that they behave. It contains no code and never imports the
   engine: core runs its rules, and the repository's tests validate it.
2. **The directory is the contract.** What a pack must prove follows from which files it has. There is no separate
   validation manifest.
3. **Every rule is proven.** By examples it must catch and let pass, or, for a rule ported from another tool, by that
   tool's recorded findings with every disagreement declared.
4. **Packs version on their own.** Each pack is its own npm package, released separately from core.

## Layout

```text
packs/<id>/
├── package.json        npm package @neralabs/rerail-pack-<id>: licence, files, core range
├── pack.json           the pack: rerail.rule-pack/v1, as today
├── README.md           what the pack covers, for people
├── LICENSE, NOTICE     the pack's licence and notice
├── rules/
│   └── <name>.json     code-rule documents (rerail.policy/v1)
├── licenses/
│   └── <project>/      upstream LICENSE (verbatim) and UPSTREAM.md (every copied or adapted file)
├── scrub.json          failures the existing rules have today, to clear before going live (when there are any)
│
│   only for packs that port rules from another tool:
├── conformance.json    the conformance pack schema of conformance/languages/pack.ts, as today
├── divergences.json    declared disagreements with the tools, as today
├── upstream/
│   └── <tool>/         the tool's copied tests (fixtures/), its recordings (*.expected.json), its source facts
│                       (source.json), the recorder
└── candidates/         ports still being evaluated: scored, never shipped
```

`<id>` is lowercase letters, digits and dashes. It is the directory name, the `id` in `pack.json`, and the npm name
`@neralabs/rerail-pack-<id>`.

### package.json

```json
{
  "name": "@neralabs/rerail-pack-tslint",
  "version": "0.1.0",
  "license": "Apache-2.0",
  "files": ["pack.json", "rules", "licenses", "LICENSE", "NOTICE", "README.md"],
  "peerDependencies": { "@neralabs/rerail": ">=0.1.0-0 <0.2.0" },
  "peerDependenciesMeta": { "@neralabs/rerail": { "optional": true } },
  "publishConfig": { "access": "restricted" }
}
```

- The core version range lives in `peerDependencies`, so npm warns when the installed core does not fit. It is a
  declaration, not an import: a pack has no `dependencies`. The `-0` admits core's prereleases (`0.1.0-next.0`).
- The peer is `optional`: `packs/*` are workspaces of this repository, whose root package is core itself, and npm
  would otherwise try to install a published core into them. npm still checks the range when core is installed.
- `files` publishes the pack and its licence records. `conformance.json`, `divergences.json`, `scrub.json`,
  `upstream/` and `candidates/` are validation material: they stay in the repository and never reach npm, which keeps
  copied upstream tests out of the package.
- The schema is `rulePackPackageSchema` in `src/rule-packs.ts`.

### pack.json

Today's `rerail.rule-pack/v1` ([format](rule-packs.md#format)): `id`, `name`, `category`, `covers`, `reason`,
`match`, `honourSuppressionComments`, and per rule `id`, `title`, `group`, optional `mode`, `on` and
`examples: { caught, passes }`. One change: a rule now names its document, `rules/<name>.json`, so a pack is
self-contained. `document` is required (`rulePackFileSchema` in `src/rule-packs.ts`); core reads each rule's document
from the pack:

```json
{ "id": "tslint:triple-equals", "document": "rules/triple-equals.json", "title": "Requires === and !==", "group": "TSLint" }
```

An example's `path` is required in a pack: it decides the file's language and must be inside the rule's scope.

### conformance.json, divergences.json, upstream/

Today's schemas from `conformance/languages/pack.ts`, moved next to the rules they validate. The runner reads a pack's
recordings from `upstream/` unless `conformance.json` sets `oracles`:

- `conformance.json`: the language family, the engine capabilities the pack declares (with exemptions and their
  evidence), the pinned tools and public corpora, and for each ported rule the upstream tool and rule it reproduces,
  how findings compare (`line` or `line+column`) and the capabilities it exercises. A tool's optional `recommended`
  lists its approved configurations and declares its source facts ([below](#source-facts-and-recordings)).
- `upstream/<tool>/fixtures.expected.json` and `corpus-<name>.expected.json`: what the tool reported on its own tests
  and on each pinned corpus, produced by the recorder in the same directory. Recorders are run by hand against the
  pinned tool versions; tools and corpus checkouts live under `.rerail/{tools,corpora}/<id>/` (gitignored).
- `upstream/<tool>/source.json`: what the tool's own source says about each ported rule, written by the same recorder
  when the tool declares `recommended`.
- `divergences.json`: each place Rerail and the tool disagree, with its kind (`upstream-wrong`, `not-comparable`,
  `rerail-unknown`) and the evidence checked. In `packs/*` it only shrinks: the [guard](#the-scrub-list-scrubjson)
  refuses new entries.

**TSLint** is scored by its own harness, `conformance/tslint/` ([README](../conformance/tslint/README.md)), because
TSLint's tests are `.lint` files with option variants that the harness fetches at a pinned commit instead of
recordings. Its files carry a `format` that names them: `conformance.json` is the golden manifest
(`rerail-tslint-golden/v1`: per shipped rule, the verdict and true positives the golden gate holds it to), and
`divergences.json` is the PASS audit (`rerail-tslint-audit/v1`: per port that passes, whether it is genuine or where
it differs from TSLint on real code). `candidates/` holds the ports not shipped, their verdict sidecars and every
rule's option-variant overrides (`candidates/<rule>/<variant>.json`). Check 2 counts a golden manifest entry as a
port entry; check 4 checks that the manifest lists exactly the pack's rules, and the golden gate
(`conformance/tslint/golden-gate.test.ts`, run by `npm run test:conformance`) scores them against TSLint's tests. So
the rule-level sub-checks of check 4 are not judged for TSLint's ports, and neither are checks 7 to 9, since TSLint
has no recorded source facts. Checks 1, 2, 5 and 6 judge the pack as any other, and check 3 does not apply to its
rules, which have no examples.

## The validation contract

Rerail validates rules in two layers. They are complementary, and they overlap only where check 4 scores a port with
the conformance runner:

- **The language conformance layer** asks whether the engine reads a language well enough, on upstream's tests and
  on real code: grammar parity (T1, with the [known parser gaps](language-packs.md#known-parser-gaps) in
  `conformance/t1/`), capability oracles, corpus differentials, host routing, the
  [language registry](language-packs.md) and the fixture tier of language packs not yet under `packs/`
  (`test/language-conformance.test.ts`). It judges the engine and its languages, never a single rule.
- **The rule validation layer** asks whether each pack rule is correct and fit to ship: checks 1 to 9 below, over
  every `packs/<id>/`. Checks 1 to 6 are the pack's own bar. Checks 7 to 9, and the sub-checks check 4 adds to that
  bar, judge a ported rule against its upstream tool's own source and tests.

Passing the language conformance layer is not enough to merge a rule: a new rule passes every check of the rule
validation layer.

Implemented: `validatePack(directory, { corpora?, corpusRoot? })` in `conformance/pack-validation.ts` runs checks 1
to 9 over one pack. It returns each check's failures that `scrub.json` does not cover, each rule's status (`ok`,
`failed`, `scrubbed` or `not-supported-yet`), and for each check the reasons it is not judged for some rules.
`test/packs.test.ts` calls it for every directory under `packs/`, with one test per check and one per rule, and has
the validator's own tests over the fixture packs in `test/fixtures/packs/`. A check test fails on an uncovered failure
and is skipped, with the reasons, when the check is not judged for some of the pack's rules. A rule test fails on an
uncovered failure, passes a `scrubbed` rule (its title lists the rule's `scrub.json` entries), and is skipped for a
rule not supported yet. `npm test` runs it with the local core and
reads only committed files: no upstream tool and no network. Running it against the newest published core inside each
pack's range is for pack CI, which does not exist yet.

```sh
npx vitest run test/packs.test.ts                         # every pack: checks 1 to 9, each rule's status
npx vitest run test/packs.test.ts -t "pack python"        # one pack
RERAIL_PACK_CORPORA=1 npx vitest run test/packs.test.ts   # check 4 on every pinned corpus too
npm run validate:packs -- --write                         # one report per pack, in .rerail/reports/
npm run validate:packs -- --pack python                   # one pack's summary, nothing written
```

Check 4 always scores ported rules on the tools' fixtures. It scores a pinned corpus when its checkout is present
under `.rerail/corpora/<id>/<corpus>` (the nearest ancestor checkout's); `RERAIL_PACK_CORPORA=1` scores every corpus
and fails on a missing checkout, and `RERAIL_PACK_CORPORA=0` scores none. To see every false positive and false
negative of one ported rule: `node --import tsx conformance/languages/cli.ts --pack <id> --rule <rule> --fixtures`.

| # | Check | Judges | Fails when |
|---|---|---|---|
| 1 | Rules | every rule | A file in `rules/` is not listed in `pack.json`; a listed `document` is missing, repeats a JSON key, does not compile against core's capabilities as the catalog compiles it, or has an `id` or compiled id other than the listing's; it has an `llm` clause; a port is out of scope for a file type its upstream tool lints (or, without source facts, an ordinary source file of its language) |
| 2 | Proof | every rule | A rule has neither `examples` nor a port entry in `conformance.json` |
| 3 | Examples | rules with examples | A `caught` example produces no violation of its rule, or a `passes` example produces one |
| 4 | Ports | ported rules and the pack | With `conformance.json`: a ported rule's violations and the tool's recordings differ outside `divergences.json`, at the declared granularity or at a finer part the recording carries; a miss is excused only as `rerail-unknown`; no upstream finding is reproduced where the port agrees completely; an upstream test of the rule is not among the fixtures; a divergence or an exemption is stale; a declared capability has neither a ported rule nor an exemption |
| 5 | Package | the pack | `package.json` misnames the pack, has `dependencies`, or publishes validation material; its licence disagrees with what `licenses/` records |
| 6 | Licences | the pack | A copied file is missing from an `UPSTREAM.md`; `reuse lint` or the licence allowlist fails |
| 7 | Provenance | ported rules and each tool | The tool's recorded pins disagree with the pack; upstream has no such rule with tests, deprecates it, or turns it on in no approved configuration |
| 8 | Generality | ported rules | The document hardcodes a name that occurs in upstream's tests but not in upstream's code, or has a regex that cannot be judged |
| 9 | Size | ported rules | The document is over 15,360 bytes and is structure written out over and over |

### Sub-checks

Every failure is filed under one sub-check (`SUB_CHECKS` in `conformance/checks.ts`). A sub-check's id is its check's
name for the check's own bar, or `<check>:<name>`. A rule-level sub-check has an outcome on every rule. A tool-level
or pack-level one fails its check and never sets a rule's status. Sub-checks marked "source facts" are judged only for
a port whose tool has recorded them. `scrub.json` may cover a failure only of a sub-check with a scrub group.

| Sub-check | Level | Judges | Source facts | Scrub group |
|---|---|---|---|---|
| `rules` | rule (pack when no rule is named) | every rule | no | never |
| `rules:deterministic` | rule | every rule | no | never |
| `rules:in-scope` | rule | ported rules | no | never |
| `proof` | rule | every rule | no | never |
| `examples` | rule | rules with examples | no | never |
| `ports:points` | rule | ported rules | no | never |
| `ports:span` | rule | ported rules | no | `fidelity` |
| `ports:unknown-misses` | rule | ported rules | no | `fidelity` |
| `ports:evidence` | rule | ported rules | no | `fidelity` |
| `ports:coverage` | rule | ported rules | yes | `fidelity` |
| `ports` | pack | the pack | no | never |
| `package` | pack | the pack | no | never |
| `licences` | pack | the pack | no | never |
| `provenance:pins` | tool | each tool with source facts | yes | never |
| `provenance:upstream-rule` | rule | ported rules | yes | `fidelity` |
| `provenance:stable` | rule | ported rules | yes | `upstream-policy` |
| `provenance:recommended` | rule | ported rules | yes | `upstream-policy` |
| `generality:names` | rule | ported rules | yes | `fidelity` |
| `generality:regexes` | rule | ported rules | yes | `fidelity` |
| `size:combinatorial-expansion` | rule | ported rules | yes | `fidelity` |
| `size:written-out-repetition` | rule | ported rules | yes | `fidelity` |

A `fidelity` failure is a defect of the port, or upstream evidence of its fidelity that check 4 does not score
(`ports:coverage`). An `upstream-policy` failure is upstream's own position on the rule: it deprecates the rule, or
turns it on in none of the configurations it recommends. The report keeps the two apart.

Precisely:

1. **Rules.** `pack.json` must parse (`rulePackFileSchema`) and its `id` must be the directory name. Each document
   must repeat no JSON key, be a valid code rule (`rerail.policy/v1`, the host's schema), and compile in the engine
   with core's `code-rule-dsl/capabilities.json` as the catalog compiles it: the schema's output in stable JSON, not
   the raw text. Its `id` and its compiled id must be the id `pack.json` lists. A file in `rules/` that `pack.json`
   does not list, or a `pack.json` that does not parse, is a pack-level failure.
   - `rules:deterministic`: no clause is an `llm` clause, which would need a model at evaluation time.
   - `rules:in-scope`, for ported rules: the compiled rule's scope contains `src/example<extension>` for every file
     extension its upstream tool's approved configurations lint (`source.json` `extensions`: Ruff's `.py`, `.pyi`;
     Bandit's `.py`; ESLint's and its plugins' `.js`, `.mjs`, `.cjs`; typescript-eslint's those and `.ts`, `.mts`,
     `.cts`, `.tsx`), as the rule validation ladder's V1 probes it. A port whose tool has no source facts is probed
     instead, for each AST language its clauses name, with every file suffix of that language's own dialects in the
     registry (a dialect another language is named for is that language's: `typescript` also reads `javascript`
     files, but a TypeScript rule need not; `php`: `.php`; `blade`: `.blade.php`), and is not probed when its clauses
     name no AST language. It does not apply to a rule proven only by examples (check 3 proves its example paths are
     in scope).
2. **Proof.** A port entry is a `conformance.json` rule whose `policy` is the rule's `document`.
3. **Examples.** Each example is evaluated alone, as a one-file repository at its `path`, through the engine (the
   `bundled-rules` test's path). Its path must be in the rule's scope, and it must produce no engine failure. A
   violation counts when its code is one of the rule's clauses' codes.
4. **Ports.** The fixture tier of the conformance runner (`conformance/languages/runner.ts`, the
   `language-conformance` test's), over the production workspace path. Rerail's violations and the recordings are
   compared in the recordings' coordinates: 1-based lines, 1-based columns in Unicode code points, and an end that is
   the position just past the finding's last character. Ports in `conformance.json` that `pack.json` does not list
   (candidates) are scored too.
   - `ports:points` is the runner's bar at the port's declared `match`, with every kind of divergence excusing: an
     unexplained false positive on any scored source, an unexplained false negative on the fixtures, an engine
     failure, a recording error the port owns, or a stale divergence. It is check 4 as it was before checks 7 to 9.
   - `ports:span`: on the fixtures, a finding `ports:points` matched must also agree on every finer part the
     recording carries for the rule (column, then `endLine` and `endColumn`), even for a port declared `line`.
     Divergences excuse as in `ports:points`, and a finding `ports:points` already failed is not failed again here.
   - `ports:unknown-misses`: an upstream fixture finding Rerail misses must not be excused only by a `rerail-unknown`
     divergence. Such an entry admits a false negative: it keeps `ports:points` passing, but not this sub-check.
     `upstream-wrong` and `not-comparable` still excuse.
   - `ports:evidence`: at least one upstream finding of the rule is reproduced, at every part the recording carries,
     in a fixture unit where the port agrees completely: nothing unmatched on either side, no engine failure other
     than an informational one, and no finding a divergence explains. A port no recorded finding exercises fails it.
   - `ports:coverage`, for a port whose tool has source facts: every upstream test of the rule (`source.json` `tests`,
     and each RuleTester case in them) is among the fixtures the sub-checks above score. The recorder lists each one it
     leaves out in the rule's `unscored`, with why: a notebook no dialect reads, a file upstream's tests lint only with
     other settings, a file or example the recorder does not copy, a case with a custom parser, type information or
     another parser's syntax (Flow), or code the pack's configuration cannot parse. A port that passes only because the
     tests it would fail on are left out fails here instead, as the rule validation ladder's V1 failed it on them. A
     test upstream skips, and a case whose code another scored case repeats, is no test lost.
   - The pack-level `ports` fails on a declared capability with neither a ported rule nor an exemption, a stale
     exemption, a recording or corpus error that no port owns, and for TSLint a golden manifest that does not list
     exactly the pack's rules.
5. **Package.** The npm name is `@neralabs/rerail-pack-<id>`. `files` must publish `pack.json`, `rules`, `LICENSE`
   and, when present, `licenses`, and no entry may cover `conformance.json`, `divergences.json`, `scrub.json`,
   `upstream` or `candidates`. The SPDX ids `license` names must be exactly Apache-2.0 plus the licence each
   `licenses/<project>/UPSTREAM.md` records on its `License: <SPDX id>` line.
6. **Licences.** Each `licenses/<project>/` holds the upstream licence text and an `UPSTREAM.md` whose table rows
   start with a pack-relative path in backticks. Every file in a subdirectory of `upstream/<tool>/` (the tool's copied
   tests: `fixtures/`, `tests/`) must be listed, and so must `upstream/<tool>/source.json`, whose names, words and
   regexes are extracted from the tool's source and tests (`REUSE.toml` annotates it with the tool's licence and
   Apache-2.0); the other files directly in `upstream/` or `upstream/<tool>/` (recordings, recorders) are Rerail's.
   Every listed file must exist, every recorded licence must be on the repository allowlist
   (`scripts/licence-allowlist.json`), and the pack needs a `LICENSE`. `reuse lint` and
   `node scripts/check-licenses.mjs` stay repository-wide gates in CI; annotate copied files in `REUSE.toml` as well,
   since `reuse lint` alone accepts a copied file the catch-all Apache-2.0 annotation covers. This is the contract's
   only licence check.
7. **Provenance**, for each port whose tool has source facts:
   - `provenance:pins`, once per tool: `upstream/<tool>/source.json` exists and is valid; its `tool` is the
     directory's name; its `version` is the tool's `version` in `conformance.json` and every recording's `version`;
     its `configurations` are the tool's `recommended`; and `<repository>@<commit>` is the fixtures recording's
     `origin`. A tool whose pins fail cannot be scrubbed rule by rule.
   - `provenance:upstream-rule`: `source.json` has an entry for the port's `upstream.rule` that lists at least one
     implementation file and one test file.
   - `provenance:stable`: upstream does not deprecate the rule (`deprecated` is null).
   - `provenance:recommended`: an approved configuration turns the rule on (`enabledBy` is not null).
8. **Generality**, for each port whose tool has source facts:
   - `generality:names`: every word the document hardcodes, in its patterns, regexes, pattern contexts and values,
     must be explained: syntax of the language (`keywords`, `commentVocabulary`), a name the rule's upstream
     implementation checks (`names.implementation`), a name a helper it delegates to checks (`sharedNames`,
     `names.shared`), or a test word one of the implementation's regexes selects. A word that occurs in upstream's
     tests (`testWords`) and is none of these is fixture-only, and fails the rule: it passes upstream's tests by
     matching the tests' own names. In a pattern context, only the words of the node the selector picks count; the
     rest is scaffolding.
   - `generality:regexes`: every regex of the document is run, as a JavaScript regex, over the copied test files in
     `upstream/<tool>/fixtures/` and over each word of upstream's test code (`testWords`) on its own, and the
     identifiers its matches touch are judged as names. A regex that touches more than ten unexplained test identifiers
     is a syntax pattern and names none; the copied files alone hold too few identifiers to tell (Ruff F601's
     `[jJ]\)*$` touches only `j` and `obj` in them), so the words stand in for the rest of upstream's test text, which
     `source.json` cannot hold (Ruff's fixtures are 6.9 MB). The words of test configurations and messages
     (`configurationWords`) count as test words for `generality:names` but are not probed, as V1 probed only test code.
     What the words lose is context: V1 ran each regex over whole texts, so a regex that needs the text around a word
     (`myHook\.use`) selected it wherever upstream's tests hold that text, and here only where the copies do. A regex
     that cannot be translated fails rather than pass unjudged.
9. **Size**, for each port whose tool has source facts. A document over 15,360 bytes is flagged and inspected; size
   alone never fails it.
   - `size:combinatorial-expansion`: a third or more of a flagged document's clauses, in canonical bytes, is
     structure written out over and over: verbatim copies of a subtree, or three or more same-shaped siblings that
     spell out a product cell by cell. Violation blocks never count as repeated.
   - `size:written-out-repetition`: one node kind occurs three or more times along a single nesting path of a flagged
     document.

### Statuses and outcomes

Every rule-level sub-check has one outcome on every rule:

| Outcome | Means |
|---|---|
| `ok` | The rule passes the sub-check. |
| `failed` | It fails, and `scrub.json` does not cover the failure; or `scrub.json` has an entry for a sub-check the rule does not fail (a stale entry: "passes; remove the scrub.json entry", or "is n/a: ..." when it does not apply). A sub-check that applies but cannot run is `failed` too, with a detail starting `not run:` (for example, check 8 on a document that does not compile). |
| `scrubbed` | It fails, and `scrub.json` covers the failure. |
| `n/a` | The sub-check does not apply to the rule: check 3 on a rule without examples, or check 4's rule-level sub-checks and checks 7 to 9 on a rule proven only by examples. |
| `not-judged` | It applies, but this layer records no evidence for it: checks 7 to 9 and `ports:coverage` for a tool without source facts ("no source facts recorded for `<tool>`"), and check 4's rule-level sub-checks for a port another harness scores (TSLint's golden manifest). |
| `not-supported-yet` | The rule's language, or a capability it reads, is not supported yet: checks 3, 4 and 7 to 9 wait for core to read it. |

A rule's status follows from its failures only: `failed` if any failure is uncovered or any entry stale, else
`scrubbed` if any failure is covered, else `not-supported-yet` or `ok`. Nothing reads `n/a`, `not-judged` or a
sub-check that did not run as a pass: an `ok` rule with a not-judged sub-check is reported as such, and the report
counts those rules and names each check that is not judged for a pack. A rule's `stoppedAt` is the lowest check with a
`failed` or `scrubbed` outcome; across a pack it gives the funnel of where rules stop.

**Languages not supported yet.** A pack may hold code rules for a language core's
[language registry](language-packs.md) lists as `planned`, or rules that read a capability their language has
`not-supported-yet`. Such a rule fails to compile with the code `language-not-supported`. The tests report it as
`not supported yet` (its rule test is skipped with the engine's reason) and do not fail: checks 1 and 2 judge it, and
checks 3, 4 and 7 to 9 skip it until core reads the language. Any other compile error still fails check 1. So a pack
for a planned language can land before the engine supports it.

### Source facts and recordings

Everything checks 4 and 7 to 9 read about upstream is recorded offline, by the tool's recorder, and committed.
`npm test` reads that JSON; it runs no upstream tool and needs no network.

- **Approved configurations.** A tool's `recommended` in `conformance.json` lists, in order, the upstream
  configurations that count as approved, named as its recorder names them: Ruff's and Bandit's `default`, ESLint's
  `recommended`, and each ESLint plugin's own. A port is recorded with the options of the first approved
  configuration that turns its rule on, and with upstream's defaults when none does. Declaring `recommended` declares
  the tool's source facts: its recorder writes `source.json`, and checks 7 to 9 judge its ports. A tool without
  `recommended` leaves them not judged; one with it but without a valid `source.json` fails `provenance:pins`.
- **Recordings.** `fixtures.expected.json` and `corpus-<name>.expected.json` hold each finding's rule, path and line,
  and the parts of its position the tool reports: `column`, and the end as `endLine` and `endColumn`. Every finding of
  one rule carries the same parts, and those are the parts `ports:span` and `ports:evidence` compare. A tool that
  reports no end for a rule records none, and Bandit, whose end column lies on a line it does not report, records
  `endColumn` without `endLine`.
- **Source facts.** `upstream/<tool>/source.json` (`upstreamSourceSchema` in `conformance/languages/pack.ts`):

  | Field | Holds |
  |---|---|
  | `tool`, `version`, `repository`, `commit`, `tag` | The pins: the tool, its release, the upstream repository, the commit the facts were read at, and the release tag the runtime was published from |
  | `configurations` | The approved configurations the facts were recorded under: the tool's `recommended` |
  | `extensions` | The file extensions those configurations lint, which `rules:in-scope` probes |
  | `keywords`, `commentVocabulary` | The language's keywords, and the words the language or the tool gives meaning inside comments (directives, pragmas, documentation tags) |
  | `testWords` | Every word of upstream's test code, for all of the tool's rules: the test files, each RuleTester case's code and expected output, and the source files the tests load |
  | `configurationWords` | The other words of upstream's tests, which `testWords` lacks: test configurations (options, settings, globals), expected messages and the other files the tests load |
  | `sharedNames` | The names the helpers every rule shares check |
  | `rules.<upstream rule>` | Per ported upstream rule: its `implementation` and `tests` (upstream paths), `deprecated` (upstream's deprecation, or null), `enabledBy` (the first approved configuration that turns it on and its options, or null), the `names` its implementation and its own helpers check, the `regexes` it matches names with, translated to JavaScript, and the upstream tests its fixtures leave out (`unscored`: test file, case or null, and the reason, from `UNSCORED_REASONS`) |

  Sets are sorted with no repeats, so two runs of a recorder write the same bytes. Licence facts are not recorded:
  check 6 is the licence check.

To re-record a tool, for a new tool version or a changed port:

1. Install the pinned tool under `.rerail/tools/<id>/` and check out upstream at the pinned commit, as the header of
   the recorder, `upstream/<tool>/generate.*`, describes. The recorder refuses a checkout at another commit, a
   modified working tree, a source tree that differs from the release tag, and a runtime at another version.
2. Run the recorder as its header says. To move a pin, change the tool's `version` in `conformance.json` and the
   recorder's pins together, and re-record every recording and `source.json` of the tool, or `provenance:pins` fails.
3. Run `npx vitest run test/packs.test.ts -t "pack <id>"`. A failure the new recording clears leaves its `scrub.json`
   entry stale: remove it. A new failure is fixed in the rule: `scrub.json` does not grow.

### The scrub list: scrub.json

Rules already in a pack may have been ported quickly, and may fail sub-checks the pack's own bar did not have. Rather
than block work on them, `packs/<id>/scrub.json` lists those failures, to be cleared before the rules go live:

```json
[
  { "rule": "python-conformance:ruff-b011-assert-false", "check": "provenance:recommended" }
]
```

- Each entry is a rule `pack.json` lists and a sub-check with a scrub group, sorted by rule, then sub-check, each pair
  once. Entries carry no reason: the report gives the failures.
- A covered failure does not fail its check: the rule is `scrubbed`. An uncovered failure fails as usual, which is
  what a new rule, a rule regressing on another sub-check, or an engine change that breaks a passing rule produces.
- An entry the rule now passes is stale and fails the sub-check, so the list only shrinks as rules are fixed.
- Checks 1, 2, 3, 5 and 6, `ports:points`, the pack-level `ports` and `provenance:pins` are never scrubbed: the pack's
  own bar never weakens, and a tool without valid source facts cannot be scrubbed rule by rule.
- A `scrub.json` that does not parse, or that names a rule `pack.json` does not list, fails check 1 for the pack.

The union of `packs/*/scrub.json` is the scrub list for going live. The report splits it into fidelity defects to fix
and upstream policy exclusions (`provenance:stable`, `provenance:recommended`) to decide on.

**The guard.** `node scripts/check-scrub.mjs <base-ref>` compares the checkout with a base commit, and CI runs it on
every pull request against the pull request's base. It refuses:

1. a `scrub.json` entry the base does not have, unless the entry's sub-check id does not occur in the base's
   `conformance/checks.ts` (a sub-check added since, which may be scrubbed on the rules that existed before it) and the
   rule is listed at the base with a byte-identical document. A pack with no `scrub.json` at the base gets no list
   for the sub-checks the base had, and deleting a `scrub.json` empties it. So a list only shrinks, and a new or
   changed rule is never scrubbed;
2. a new entry in a `packs/*/divergences.json` (TSLint's audit, which carries a `format`, is not a divergence list);
3. a new rule checks 4 and 7 to 9 cannot judge: a port whose tool declares no source facts, or any rule of a pack
   whose `conformance.json` is a golden manifest. A new port always faces every check. A new rule without a port is
   admitted in any other pack, with or without a `conformance.json`: its examples prove it (checks 2 and 3), and checks
   4 and 7 to 9 are `n/a` for it. This reading is a choice: a stricter guard would refuse every new rule in a pack
   none of whose tools declares source facts (`php`, `blade`), so that no port lands there as a rule proven by
   examples;
4. taking an existing rule out of their reach: an existing port losing its `conformance.json` entry (or the pack its
   `conformance.json`), a tool dropping the `recommended` the base has, a port the base's facts judged becoming one
   they do not, or a rule proven by examples becoming a port they cannot judge.

### The report

`npm run validate:packs -- --write` validates every pack (`--pack <id>`, repeatable, for some) and writes
`.rerail/reports/<id>-validation.json` and `<id>-validation.md` (`.rerail/` is gitignored). It refuses to write while
`RERAIL_ENGINE_PATH` or `RERAIL_CODE_RULE_PROFILE` is set, so a report is always of the checkout's own engine, and
exits 1 when a pack fails. The JSON is `rerail.pack-validation/v1` (`validationReportSchema` in
`conformance/validation-report.ts`); the markdown renders the same facts. A report has no date and no timing, so two
runs on the same commit write the same bytes, and it names the engine that ran as its commit and whether the checkout
was dirty. It holds:

- each tool's pins and approved configurations, or that it has none;
- the checks not judged for the pack, and the tool-level and pack-level failures;
- the scrub entries that cover a failure, grouped into fidelity defects and upstream policy exclusions;
- a summary: how many rules are `ok`, `failed`, `scrubbed` and `not-supported-yet`, how many have a not-judged
  sub-check, where rules stop, and how many fail or are scrubbed on each sub-check;
- per rule: its document's digest and size, its port, status and `stoppedAt`, every rule-level sub-check's outcome and
  details, the words check 8 classed, check 9's size metrics, and check 4's totals with the differing fixture files.

CI enforces `scrub.json`, not the reports: a report is for reading where a pack stands.

### Not yet

- **Source facts for the other tools.** Recorders that write `source.json` and finding ends for the `php` pack's
  tools (PHPStan, Psalm, PHP_CodeSniffer), the `blade` pack's (Bladestan, blade-parser, TLint) and TSLint. Until then
  checks 7 to 9 are not judged for those packs, TSLint's ports stay with its golden gate, and the guard refuses new
  ports there.
- **Scoring the unscored tests.** `ports:coverage` names each upstream test the fixtures leave out: notebooks, which
  wait for a dialect that reads `.ipynb`; files and cases upstream lints only under other settings, or written for
  another parser (Flow), which wait for a per-case environment; and code the pack's configuration cannot parse. A file
  upstream lints under the approved configuration is copied, which scores every port of the tool on it.
- **Real code.** Check 4 scores a pinned corpus only when its checkout is present, and CI fetches none. Still to
  come: a real-code sub-check every port is held to on every pinned corpus, with the corpora fetched in CI; and gap
  candidates, where independent parsers judge each file the engine cannot parse, so that a parse failure is put down
  to the file or to the engine.
- **Open sub-checks.** `rules:dialect-kinds`: a node kind a rule names must exist in every dialect it runs on (a
  TypeScript-only kind in a rule that also runs on JavaScript silently never matches there). It needs the engine to
  list each grammar's node kinds. And a rename check: renaming the identifiers of upstream's tests must leave a port's
  score unchanged. The guard admits each as a new sub-check when it lands.
- **A public `rerail pack check` command**, for packs written outside this repository, which could wrap the same
  checks.

## Versioning and release

| Change | Version bump |
|---|---|
| A false positive or false negative fixed; wording, examples or recordings changed | patch |
| A rule added; the core range widened | minor |
| A rule removed or renamed; a default mode made stricter; the core range narrowed | major |

Pack versions are managed with [changesets](https://github.com/changesets/changesets): a pull request that changes a
pack adds a changeset (`npx changeset`) naming the pack's package and the bump from the table above, with a one-line
summary for users. Nobody edits a pack's `version` or `CHANGELOG.md` by hand: `npx changeset version` applies the
pending changesets, bumps each pack and writes its `CHANGELOG.md` when packs are released. Changesets cover only the
packs: the core package at the repository root keeps its `v<version>` tag release and the root `CHANGELOG.md`, and a
changeset naming `@neralabs/rerail` is refused. A pack's first pull request adds it at `0.1.0` with a `minor` changeset
for its initial rules.

Each pack is released on its own: its package publishes `packs/<id>/` through the release workflow and trusted
publishing, under `next` first and `latest` once promoted, `restricted` until launch like core. Publishing from the
release workflow is not wired yet; until it is, changesets accumulate and nothing is published.

## Licensing

Rerail's own material in a pack is Apache-2.0. Material copied or adapted from an upstream project keeps its licence:
listed in `licenses/<project>/UPSTREAM.md` and annotated in the repository's `REUSE.toml`. The pack's `license`
names every licence its published files carry (for example `Apache-2.0 AND MIT` for rules adapted from an MIT tool).
The pack's rule documents (`rules/`, and the rule documents under `candidates/`) are annotated in `REUSE.toml` with
that same expression, `Apache-2.0 AND <upstream>`, so what `reuse lint` reports for them matches what the package
publishes; whether adapted rule documents need the upstream licence is an open legal question. Only licences on the
repository allowlist are accepted.

## Moving to this layout

| Today | Becomes |
|---|---|
| `policies/rule-packs/tslint.json`, `code-rule-dsl/rules/opt-in/tslint/` (shipped in core) | `packs/tslint/pack.json` and `rules/`, still shipped in core's package until core loads installed packs |
| `conformance/tslint/rules/**` (option variants, ports not shipped) | `packs/tslint/candidates/` |
| `conformance/tslint/golden.json` and the audit in `conformance/tslint/audit.ts` | `packs/tslint/conformance.json` (golden manifest) and `divergences.json` (PASS audit) |
| TSLint's upstream tests, fetched at a pinned commit by `conformance/tslint/fixtures.ts` | Not moved: still fetched into `tmp/tslint-fixtures/`; a recording under `packs/tslint/upstream/tslint/` would let the language runner score TSLint |
| `conformance/languages/<lang>/pack.json`, `rules/`, `divergences.json` | `packs/<lang>/conformance.json`, `rules/` or `candidates/`, `divergences.json`, plus a `pack.json` |
| `fixtures/<family>/rules/<tool>/` recordings, copied tests and recorders | `packs/<lang>/upstream/<tool>/`; for a language without rules yet, `fixtures/<family>/upstream/<tool>/` |
| `fixtures/<family>/{candidates,probes,policies}/` | `packs/<lang>/candidates/` |
| `fixtures/<family>/licenses/<project>/` for rule tools | `packs/<lang>/licenses/<project>/`, keeping only the rule tests' rows; rows for engine oracles stay in `fixtures/` |
| `conformance/languages/` runner and scorer | Test support in core that the pack tests call: `conformance/pack-validation.ts` |
| Everything else under `fixtures/` (parsing, scope, values, calls oracles) | Stays: engine tests, not pack validation |

Moved: `python`, `php`, `blade`, `typescript`, `react`, `node` and `next`, each from `conformance/languages/<id>/`,
`fixtures/<id>/rules/`, its candidates and probes, and the rule rows of its licence records. The ESLint recorder they
share is `conformance/recorders/eslint-record.mjs`, and the TypeScript grammar census is `conformance/t1/typescript-syntax.ts`.
TSLint moved to `packs/tslint/` and still ships inside core's package; its harness stays in `conformance/tslint/` as
test support. The complexity rules (`code-rule-dsl/rules/opt-in/complexity/`, `policies/rule-packs/complexity.json`)
moved to `packs/complexity/` the same way; their lizard parity test stays in `test/complexity-rules.test.ts`. Not
moved yet: `rust`, `sql` and `vue`, which have upstream
recordings but no rules: they stay in `fixtures/<id>/upstream/` beside each language's draft manifest
(`fixtures/rust/upstream/conformance.draft.json`, `fixtures/sql/upstream/conformance.draft.json`,
`fixtures/vue/upstream/rules.json`) until their packs gain rules, when the draft becomes `packs/<id>/conformance.json`
and `upstream/` moves to `packs/<id>/upstream/`.
